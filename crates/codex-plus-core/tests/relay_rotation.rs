use codex_plus_core::relay_rotation::{
    RelayRotationSelector, RotationContext, RotationEvent, RouteMatchInfo, SelectionError,
    fallback_relays_after, match_route_pattern, record_relay_request_failure, route_match_outcomes,
    select_relay_for_probe, select_relay_for_request,
};
use codex_plus_core::settings::{
    AggregateRelayMember, AggregateRelayProfile, AggregateRelayRoute, AggregateRelayStrategy,
    BackendSettings, RelayMode, RelayProfile, RelaySessionProvider,
};
use std::sync::{Mutex, MutexGuard, OnceLock};

fn global_selector_test_lock() -> MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn profile(id: &str) -> RelayProfile {
    RelayProfile {
        id: id.to_string(),
        name: id.to_string(),
        base_url: format!("https://{id}.example/v1"),
        api_key: format!("sk-{id}"),
        ..RelayProfile::default()
    }
}

fn aggregate(strategy: AggregateRelayStrategy) -> AggregateRelayProfile {
    AggregateRelayProfile {
        id: "agg".to_string(),
        name: "聚合".to_string(),
        session_provider: RelaySessionProvider::Custom,
        strategy,
        members: vec![
            AggregateRelayMember {
                relay_id: "relay-a".to_string(),
                weight: 1,
            },
            AggregateRelayMember {
                relay_id: "relay-b".to_string(),
                weight: 2,
            },
            AggregateRelayMember {
                relay_id: "relay-c".to_string(),
                weight: 1,
            },
        ],
        routes: Vec::new(),
    }
}

fn aggregate_with_id(id: &str, strategy: AggregateRelayStrategy) -> AggregateRelayProfile {
    AggregateRelayProfile {
        id: id.to_string(),
        name: "聚合".to_string(),
        session_provider: RelaySessionProvider::Custom,
        strategy,
        members: vec![
            AggregateRelayMember {
                relay_id: "relay-a".to_string(),
                weight: 1,
            },
            AggregateRelayMember {
                relay_id: "relay-b".to_string(),
                weight: 2,
            },
        ],
        routes: Vec::new(),
    }
}

fn settings(strategy: AggregateRelayStrategy) -> BackendSettings {
    BackendSettings {
        relay_profiles: vec![
            profile("relay-a"),
            profile("relay-b"),
            profile("relay-c"),
            RelayProfile {
                id: "agg".to_string(),
                name: "聚合".to_string(),
                relay_mode: RelayMode::Aggregate,
                ..RelayProfile::default()
            },
        ],
        aggregate_relay_profiles: vec![aggregate(strategy)],
        active_relay_id: "agg".to_string(),
        active_aggregate_relay_id: "agg".to_string(),
        ..BackendSettings::default()
    }
}

#[test]
fn failover_keeps_current_provider_until_failure_then_moves_to_next_member() {
    let settings = settings(AggregateRelayStrategy::Failover);
    let mut selector = RelayRotationSelector::from_settings(&settings).unwrap();

    let first = selector
        .select(&settings, RotationContext::for_conversation("chat-1"))
        .unwrap();
    selector.record_event(RotationEvent::Success);
    let second = selector
        .select(&settings, RotationContext::for_conversation("chat-1"))
        .unwrap();
    selector.record_event(RotationEvent::Failure);
    let third = selector
        .select(&settings, RotationContext::for_conversation("chat-1"))
        .unwrap();

    assert_eq!(first.id, "relay-a");
    assert_eq!(second.id, "relay-a");
    assert_eq!(third.id, "relay-b");
}

#[test]
fn conversation_rotation_sticks_each_conversation_to_a_stable_member() {
    let settings = settings(AggregateRelayStrategy::ConversationRoundRobin);
    let mut selector = RelayRotationSelector::from_settings(&settings).unwrap();

    let chat_a_first = selector
        .select(&settings, RotationContext::for_conversation("chat-a"))
        .unwrap();
    let chat_a_second = selector
        .select(&settings, RotationContext::for_conversation("chat-a"))
        .unwrap();
    let chat_b_first = selector
        .select(&settings, RotationContext::for_conversation("chat-b"))
        .unwrap();

    assert_eq!(chat_a_first.id, "relay-a");
    assert_eq!(chat_a_second.id, "relay-a");
    assert_eq!(chat_b_first.id, "relay-b");
}

#[test]
fn request_rotation_advances_on_every_request() {
    let settings = settings(AggregateRelayStrategy::RequestRoundRobin);
    let mut selector = RelayRotationSelector::from_settings(&settings).unwrap();

    let selected = (0..5)
        .map(|_| {
            selector
                .select(&settings, RotationContext::default())
                .unwrap()
                .id
        })
        .collect::<Vec<_>>();

    assert_eq!(
        selected,
        vec!["relay-a", "relay-b", "relay-c", "relay-a", "relay-b"]
    );
}

#[test]
fn weighted_rotation_repeats_members_by_configured_weight() {
    let settings = settings(AggregateRelayStrategy::WeightedRoundRobin);
    let mut selector = RelayRotationSelector::from_settings(&settings).unwrap();

    let selected = (0..6)
        .map(|_| {
            selector
                .select(&settings, RotationContext::default())
                .unwrap()
                .id
        })
        .collect::<Vec<_>>();

    assert_eq!(
        selected,
        vec![
            "relay-a", "relay-b", "relay-b", "relay-c", "relay-a", "relay-b"
        ]
    );
}

#[test]
fn aggregate_members_must_reference_existing_relay_profiles() {
    let mut settings = settings(AggregateRelayStrategy::RequestRoundRobin);
    settings.aggregate_relay_profiles[0]
        .members
        .push(AggregateRelayMember {
            relay_id: "missing-relay".to_string(),
            weight: 1,
        });

    let mut selector = RelayRotationSelector::from_settings(&settings).unwrap();
    // 前三次轮转到有效成员，第四次才遇到不存在的成员
    for expected in ["relay-a", "relay-b", "relay-c"] {
        assert_eq!(
            selector
                .select(&settings, RotationContext::default())
                .unwrap()
                .id,
            expected
        );
    }
    // 轮转到不存在的成员时报 UnknownMemberRelay
    let error = selector
        .select(&settings, RotationContext::default())
        .unwrap_err();
    assert_eq!(
        error,
        SelectionError::UnknownMemberRelay {
            aggregate_id: "agg".to_string(),
            relay_id: "missing-relay".to_string()
        }
    );
}

#[test]
fn aggregate_with_one_member_is_allowed_without_rotation() {
    let mut settings = settings(AggregateRelayStrategy::RequestRoundRobin);
    settings.aggregate_relay_profiles[0].members.truncate(1);

    let mut selector = RelayRotationSelector::from_settings(&settings).unwrap();
    let first = selector
        .select(&settings, RotationContext::default())
        .unwrap();
    let second = selector
        .select(&settings, RotationContext::default())
        .unwrap();

    assert_eq!(first.id, "relay-a");
    assert_eq!(second.id, "relay-a");
}

#[test]
fn aggregate_members_must_be_api_capable_relay_profiles() {
    let mut settings = settings(AggregateRelayStrategy::WeightedRoundRobin);
    settings.relay_profiles.push(RelayProfile {
        id: "official-login".to_string(),
        name: "官方登录".to_string(),
        base_url: String::new(),
        api_key: String::new(),
        ..RelayProfile::default()
    });
    settings.aggregate_relay_profiles[0]
        .members
        .push(AggregateRelayMember {
            relay_id: "official-login".to_string(),
            weight: 1,
        });

    let mut selector = RelayRotationSelector::from_settings(&settings).unwrap();
    // 权重调度前 4 次都是有效成员（relay-a, relay-b, relay-b, relay-c）
    for expected in ["relay-a", "relay-b", "relay-b", "relay-c"] {
        assert_eq!(
            selector
                .select(&settings, RotationContext::default())
                .unwrap()
                .id,
            expected
        );
    }
    // 轮转到无 key 成员时报 InvalidMemberRelay
    let error = selector
        .select(&settings, RotationContext::default())
        .unwrap_err();
    assert_eq!(
        error,
        SelectionError::InvalidMemberRelay {
            aggregate_id: "agg".to_string(),
            relay_id: "official-login".to_string()
        }
    );
}

#[test]
fn aggregate_members_accept_no_auth_pure_api_profiles() {
    let mut settings = settings(AggregateRelayStrategy::RequestRoundRobin);
    settings.relay_profiles[0] = RelayProfile {
        id: "relay-a".to_string(),
        name: "relay-a".to_string(),
        base_url: "https://relay-a.example/v1".to_string(),
        relay_mode: RelayMode::PureApi,
        no_auth: true,
        api_key: String::new(),
        ..RelayProfile::default()
    };

    let mut selector = RelayRotationSelector::from_settings(&settings).unwrap();
    let selected = selector
        .select(&settings, RotationContext::default())
        .unwrap();

    assert_eq!(selected.id, "relay-a");
    assert!(selected.uses_no_auth());
}

#[test]
fn select_relay_for_request_uses_active_relay_id_as_aggregate_source_of_truth() {
    let _guard = global_selector_test_lock();
    let mut settings = settings(AggregateRelayStrategy::WeightedRoundRobin);
    settings.active_relay_id = "agg".to_string();
    settings.active_aggregate_relay_id.clear();

    let selected = select_relay_for_request(&settings, RotationContext::default()).unwrap();

    assert_eq!(selected.id, "relay-a");
}

#[test]
fn select_relay_for_request_ignores_stale_active_aggregate_id_for_regular_relay() {
    let _guard = global_selector_test_lock();
    let mut settings = settings(AggregateRelayStrategy::WeightedRoundRobin);
    settings.active_relay_id = "relay-b".to_string();
    settings.active_aggregate_relay_id = "agg".to_string();

    let selected = select_relay_for_request(&settings, RotationContext::default()).unwrap();

    assert_eq!(selected.id, "relay-b");
}

#[test]
fn select_relay_for_request_resets_rotation_after_switching_to_regular_relay() {
    let _guard = global_selector_test_lock();
    let mut settings = settings(AggregateRelayStrategy::RequestRoundRobin);
    settings.active_relay_id = "agg".to_string();

    let first = select_relay_for_request(&settings, RotationContext::default()).unwrap();
    let mut regular_settings = settings.clone();
    regular_settings.active_relay_id = "relay-c".to_string();
    regular_settings.active_aggregate_relay_id.clear();
    let regular = select_relay_for_request(&regular_settings, RotationContext::default()).unwrap();
    let after_reselect = select_relay_for_request(&settings, RotationContext::default()).unwrap();

    assert_eq!(first.id, "relay-a");
    assert_eq!(regular.id, "relay-c");
    assert_eq!(after_reselect.id, "relay-a");
}

#[test]
fn record_relay_request_failure_advances_global_failover_selector() {
    let _guard = global_selector_test_lock();
    let aggregate_id = "agg-global-failure";
    let settings = BackendSettings {
        relay_profiles: vec![
            profile("relay-a"),
            profile("relay-b"),
            RelayProfile {
                id: aggregate_id.to_string(),
                name: "聚合".to_string(),
                relay_mode: RelayMode::Aggregate,
                ..RelayProfile::default()
            },
        ],
        aggregate_relay_profiles: vec![aggregate_with_id(
            aggregate_id,
            AggregateRelayStrategy::Failover,
        )],
        active_relay_id: aggregate_id.to_string(),
        active_aggregate_relay_id: aggregate_id.to_string(),
        ..BackendSettings::default()
    };

    let first = select_relay_for_request(&settings, RotationContext::default()).unwrap();
    record_relay_request_failure(&settings);
    let second = select_relay_for_request(&settings, RotationContext::default()).unwrap();

    assert_eq!(first.id, "relay-a");
    assert_eq!(second.id, "relay-b");
}

#[test]
fn select_relay_for_probe_does_not_advance_request_rotation() {
    let _guard = global_selector_test_lock();
    let aggregate_id = "agg-probe";
    let settings = BackendSettings {
        relay_profiles: vec![
            profile("relay-a"),
            profile("relay-b"),
            RelayProfile {
                id: aggregate_id.to_string(),
                name: "聚合".to_string(),
                relay_mode: RelayMode::Aggregate,
                ..RelayProfile::default()
            },
        ],
        aggregate_relay_profiles: vec![aggregate_with_id(
            aggregate_id,
            AggregateRelayStrategy::RequestRoundRobin,
        )],
        active_relay_id: aggregate_id.to_string(),
        active_aggregate_relay_id: aggregate_id.to_string(),
        ..BackendSettings::default()
    };

    let first_probe = select_relay_for_probe(&settings).unwrap();
    let second_probe = select_relay_for_probe(&settings).unwrap();
    let first_request = select_relay_for_request(&settings, RotationContext::default()).unwrap();
    let second_request = select_relay_for_request(&settings, RotationContext::default()).unwrap();

    assert_eq!(first_probe.id, "relay-a");
    assert_eq!(second_probe.id, "relay-a");
    assert_eq!(first_request.id, "relay-a");
    assert_eq!(second_request.id, "relay-b");
}

#[test]
fn fallback_relays_after_returns_remaining_aggregate_members_after_current_then_wraps() {
    let settings = settings(AggregateRelayStrategy::RequestRoundRobin);

    let fallbacks = fallback_relays_after(&settings, "relay-b").unwrap();

    assert_eq!(
        fallbacks
            .iter()
            .map(|profile| profile.id.as_str())
            .collect::<Vec<_>>(),
        vec!["relay-c", "relay-a"]
    );
}

#[test]
fn fallback_relays_after_regular_relay_returns_empty_candidates() {
    let mut settings = settings(AggregateRelayStrategy::RequestRoundRobin);
    settings.active_relay_id = "relay-a".to_string();

    let fallbacks = fallback_relays_after(&settings, "relay-a").unwrap();

    assert!(fallbacks.is_empty());
}

#[test]
fn select_relay_for_request_rebuilds_selector_when_active_aggregate_changes() {
    let _guard = global_selector_test_lock();
    let aggregate_id = "agg-refresh";
    let mut settings = BackendSettings {
        relay_profiles: vec![
            profile("relay-a"),
            profile("relay-b"),
            RelayProfile {
                id: aggregate_id.to_string(),
                name: "聚合".to_string(),
                relay_mode: RelayMode::Aggregate,
                ..RelayProfile::default()
            },
        ],
        aggregate_relay_profiles: vec![aggregate_with_id(
            aggregate_id,
            AggregateRelayStrategy::Failover,
        )],
        active_relay_id: aggregate_id.to_string(),
        active_aggregate_relay_id: aggregate_id.to_string(),
        ..BackendSettings::default()
    };

    let first = select_relay_for_request(&settings, RotationContext::default()).unwrap();
    settings.aggregate_relay_profiles[0].strategy = AggregateRelayStrategy::WeightedRoundRobin;

    let selected = (0..3)
        .map(|_| {
            select_relay_for_request(&settings, RotationContext::default())
                .unwrap()
                .id
        })
        .collect::<Vec<_>>();

    assert_eq!(first.id, "relay-a");
    assert_eq!(selected, vec!["relay-a", "relay-b", "relay-b"]);
}

fn route(pattern: &str, relay_id: &str, priority: u32) -> AggregateRelayRoute {
    AggregateRelayRoute {
        pattern: pattern.to_string(),
        relay_id: relay_id.to_string(),
        priority,
    }
}

fn context(model: Option<&str>) -> RotationContext {
    RotationContext {
        conversation_id: None,
        model: model.map(str::to_owned),
    }
}

#[test]
fn route_rule_matches_model_and_selects_target_member() {
    let mut settings = settings(AggregateRelayStrategy::Failover);
    settings.aggregate_relay_profiles[0].routes = vec![route("deepseek-*", "relay-b", 0)];
    let mut selector = RelayRotationSelector::from_settings(&settings).unwrap();

    let selected = selector
        .select(&settings, context(Some("deepseek-chat")))
        .unwrap();

    assert_eq!(selected.id, "relay-b");
}

#[test]
fn route_priority_descending_and_stable_order_within_same_priority() {
    let mut settings = settings(AggregateRelayStrategy::Failover);
    settings.aggregate_relay_profiles[0].routes = vec![
        route("*", "relay-a", 0),
        route("deepseek-*", "relay-b", 10),
        route("deepseek-chat", "relay-c", 5),
    ];
    let mut selector = RelayRotationSelector::from_settings(&settings).unwrap();
    let selected = selector
        .select(&settings, context(Some("deepseek-chat")))
        .unwrap();
    assert_eq!(selected.id, "relay-b");

    // 同 priority 按数组顺序取第一条（稳定优先）
    settings.aggregate_relay_profiles[0].routes =
        vec![route("*", "relay-a", 5), route("deepseek-*", "relay-b", 5)];
    let mut selector = RelayRotationSelector::from_settings(&settings).unwrap();
    let selected = selector
        .select(&settings, context(Some("deepseek-chat")))
        .unwrap();
    assert_eq!(selected.id, "relay-a");
}

#[test]
fn route_matching_is_case_insensitive() {
    let mut settings = settings(AggregateRelayStrategy::Failover);
    settings.aggregate_relay_profiles[0].routes = vec![route("DeepSeek-*", "relay-b", 0)];
    let mut selector = RelayRotationSelector::from_settings(&settings).unwrap();

    let selected = selector
        .select(&settings, context(Some("deepseek-chat")))
        .unwrap();

    assert_eq!(selected.id, "relay-b");
}

#[test]
fn route_wildcard_pattern_matrix() {
    // 前缀
    assert!(match_route_pattern("deepseek-*", "deepseek-chat"));
    assert!(!match_route_pattern("deepseek-*", "glm-4"));
    // 后缀
    assert!(match_route_pattern("*-v3", "glm-4-v3"));
    assert!(!match_route_pattern("*-v3", "glm-4"));
    // 任意位置
    assert!(match_route_pattern("*gpt*", "x-gpt-5-y"));
    // 多 * 按序且不重叠
    assert!(match_route_pattern("a*b*c", "a1b2c"));
    assert!(!match_route_pattern("a*b*c", "ab"));
    assert!(!match_route_pattern("a*b*c", "ac"));
    assert!(match_route_pattern("a*a", "aa"));
    assert!(!match_route_pattern("a*a", "a"));
    // 仅 *
    assert!(match_route_pattern("*", "anything"));
    // 无 * 精确匹配（大小写不敏感）
    assert!(match_route_pattern("DeepSeek-Chat", "deepseek-chat"));
    assert!(!match_route_pattern("deepseek-chat", "deepseek-chat-extra"));
    // 空白 pattern 不参与匹配
    assert!(!match_route_pattern("   ", "deepseek-chat"));
}

#[test]
fn route_unmatched_falls_back_to_strategy() {
    let mut settings = settings(AggregateRelayStrategy::Failover);
    settings.aggregate_relay_profiles[0].routes = vec![route("deepseek-*", "relay-b", 0)];
    let mut selector = RelayRotationSelector::from_settings(&settings).unwrap();

    let selected = selector.select(&settings, context(Some("glm-4"))).unwrap();

    assert_eq!(selected.id, "relay-a");
}

#[test]
fn route_empty_routes_preserves_legacy_strategy_behavior() {
    let settings = settings(AggregateRelayStrategy::Failover);
    let mut selector = RelayRotationSelector::from_settings(&settings).unwrap();

    let selected = selector
        .select(&settings, context(Some("deepseek-chat")))
        .unwrap();

    assert_eq!(selected.id, "relay-a");
}

#[test]
fn route_invalid_target_relay_is_skipped_and_falls_back_to_strategy() {
    let mut settings = settings(AggregateRelayStrategy::Failover);
    settings.relay_profiles.push(RelayProfile {
        id: "relay-empty".to_string(),
        name: "empty".to_string(),
        ..RelayProfile::default()
    });
    // 全部无效（不存在 / 缺 base_url / 缺 api_key）→ 走 strategy
    settings.aggregate_relay_profiles[0].routes = vec![
        route("deepseek-*", "ghost", 10),
        route("glm-*", "relay-empty", 5),
    ];
    let mut selector = RelayRotationSelector::from_settings(&settings).unwrap();
    let selected = selector
        .select(&settings, context(Some("deepseek-chat")))
        .unwrap();
    assert_eq!(selected.id, "relay-a");

    // 第一条无效，后续有效规则生效
    settings.aggregate_relay_profiles[0].routes = vec![
        route("deepseek-*", "ghost", 10),
        route("deepseek-chat", "relay-b", 5),
    ];
    let mut selector = RelayRotationSelector::from_settings(&settings).unwrap();
    let selected = selector
        .select(&settings, context(Some("deepseek-chat")))
        .unwrap();
    assert_eq!(selected.id, "relay-b");
}

#[test]
fn route_blank_pattern_is_skipped() {
    let mut settings = settings(AggregateRelayStrategy::Failover);
    settings.aggregate_relay_profiles[0].routes = vec![route("   ", "relay-b", 10)];
    let mut selector = RelayRotationSelector::from_settings(&settings).unwrap();
    let selected = selector
        .select(&settings, context(Some("deepseek-chat")))
        .unwrap();
    assert_eq!(selected.id, "relay-a");
}

#[test]
fn route_not_applied_when_model_is_missing() {
    let mut settings = settings(AggregateRelayStrategy::Failover);
    settings.aggregate_relay_profiles[0].routes = vec![route("*", "relay-b", 10)];
    let mut selector = RelayRotationSelector::from_settings(&settings).unwrap();

    let selected = selector
        .select(&settings, RotationContext::default())
        .unwrap();

    assert_eq!(selected.id, "relay-a");
}

#[test]
fn route_matched_request_keeps_member_order_fallback() {
    let mut settings = settings(AggregateRelayStrategy::Failover);
    settings.aggregate_relay_profiles[0].routes = vec![route("deepseek-*", "relay-b", 0)];
    let mut selector = RelayRotationSelector::from_settings(&settings).unwrap();

    let selected = selector
        .select(&settings, context(Some("deepseek-chat")))
        .unwrap();
    assert_eq!(selected.id, "relay-b");

    let fallbacks = fallback_relays_after(&settings, &selected.id).unwrap();
    assert_eq!(
        fallbacks
            .iter()
            .map(|profile| profile.id.as_str())
            .collect::<Vec<_>>(),
        vec!["relay-c", "relay-a"]
    );
}

#[test]
fn route_match_outcomes_reports_events_for_logging() {
    let mut settings = settings(AggregateRelayStrategy::Failover);
    settings.aggregate_relay_profiles[0].routes = vec![
        route("deepseek-*", "ghost", 10),
        route("glm-*", "relay-b", 5),
    ];
    let outcomes = route_match_outcomes(&settings, Some("deepseek-chat"));
    assert_eq!(
        outcomes,
        vec![
            RouteMatchInfo::SkippedInvalidRelay {
                relay_id: "ghost".to_string(),
                pattern: "deepseek-*".to_string(),
            },
            RouteMatchInfo::Unmatched,
        ]
    );

    settings.aggregate_relay_profiles[0].routes = vec![route("deepseek-*", "relay-b", 5)];
    let outcomes = route_match_outcomes(&settings, Some("deepseek-chat"));
    assert_eq!(
        outcomes,
        vec![RouteMatchInfo::Matched {
            relay_id: "relay-b".to_string(),
            pattern: "deepseek-*".to_string(),
        }]
    );

    assert!(route_match_outcomes(&settings, None).is_empty());
    assert!(route_match_outcomes(&settings, Some("  ")).is_empty());
}

#[test]
fn route_target_outside_aggregate_members_is_skipped_even_if_relay_exists() {
    let mut settings = settings(AggregateRelayStrategy::Failover);
    // relay-standalone 存在于 relay_profiles，但不在聚合 members 内
    settings.relay_profiles.push(profile("relay-standalone"));
    settings.aggregate_relay_profiles[0].routes = vec![
        route("deepseek-*", "relay-standalone", 10),
        route("deepseek-chat", "relay-b", 5),
    ];

    // 非成员（区别于 ghost 的不存在）→ SkippedInvalidRelay，继续匹配后续有效规则
    let outcomes = route_match_outcomes(&settings, Some("deepseek-chat"));
    assert_eq!(
        outcomes,
        vec![
            RouteMatchInfo::SkippedInvalidRelay {
                relay_id: "relay-standalone".to_string(),
                pattern: "deepseek-*".to_string(),
            },
            RouteMatchInfo::Matched {
                relay_id: "relay-b".to_string(),
                pattern: "deepseek-chat".to_string(),
            },
        ]
    );

    let mut selector = RelayRotationSelector::from_settings(&settings).unwrap();
    let selected = selector
        .select(&settings, context(Some("deepseek-chat")))
        .unwrap();
    assert_eq!(selected.id, "relay-b");

    // 全部规则指向非成员 → 走原 strategy
    settings.aggregate_relay_profiles[0].routes =
        vec![route("deepseek-*", "relay-standalone", 10)];
    let mut selector = RelayRotationSelector::from_settings(&settings).unwrap();
    let selected = selector
        .select(&settings, context(Some("deepseek-chat")))
        .unwrap();
    assert_eq!(selected.id, "relay-a");
}

#[test]
fn select_with_outcomes_reports_route_outcomes_alongside_selection() {
    let mut settings = settings(AggregateRelayStrategy::Failover);
    settings.aggregate_relay_profiles[0].routes = vec![route("deepseek-*", "relay-b", 5)];
    let mut selector = RelayRotationSelector::from_settings(&settings).unwrap();

    // 路由命中：select_with_outcomes 与 select 选择一致，同时返回 Matched 过程结果
    let (relay, outcomes) = selector
        .select_with_outcomes(&settings, context(Some("deepseek-chat")))
        .unwrap();
    assert_eq!(relay.id, "relay-b");
    assert_eq!(
        outcomes,
        vec![RouteMatchInfo::Matched {
            relay_id: "relay-b".to_string(),
            pattern: "deepseek-*".to_string(),
        }]
    );

    // 路由未命中：select_with_outcomes 走原 strategy，并返回 Unmatched
    let (relay, outcomes) = selector
        .select_with_outcomes(&settings, context(Some("glm-4")))
        .unwrap();
    assert_eq!(relay.id, "relay-a");
    assert_eq!(outcomes, vec![RouteMatchInfo::Unmatched]);
}

#[test]
fn select_relay_for_request_applies_route_rules_via_global_selector() {
    let _guard = global_selector_test_lock();
    let aggregate_id = "agg-route-global";
    let mut aggregate = aggregate_with_id(aggregate_id, AggregateRelayStrategy::Failover);
    aggregate.routes = vec![route("deepseek-*", "relay-b", 0)];
    let settings = BackendSettings {
        relay_profiles: vec![
            profile("relay-a"),
            profile("relay-b"),
            RelayProfile {
                id: aggregate_id.to_string(),
                name: "聚合".to_string(),
                relay_mode: RelayMode::Aggregate,
                ..RelayProfile::default()
            },
        ],
        aggregate_relay_profiles: vec![aggregate],
        active_relay_id: aggregate_id.to_string(),
        active_aggregate_relay_id: aggregate_id.to_string(),
        ..BackendSettings::default()
    };

    let selected = select_relay_for_request(&settings, context(Some("deepseek-chat"))).unwrap();

    assert_eq!(selected.id, "relay-b");
}

#[test]
fn aggregate_route_serializes_with_camel_case_fields() {
    let profile = AggregateRelayProfile {
        id: "agg".to_string(),
        name: "聚合".to_string(),
        session_provider: RelaySessionProvider::Custom,
        strategy: AggregateRelayStrategy::Failover,
        members: Vec::new(),
        routes: vec![route("deepseek-*", "relay-b", 10)],
    };

    let serialized = serde_json::to_string(&profile).unwrap();

    assert!(serialized.contains(r#""pattern":"deepseek-*""#));
    assert!(serialized.contains(r#""relayId":"relay-b""#));
    assert!(serialized.contains(r#""priority":10"#));
}

#[test]
fn aggregate_profile_without_routes_deserializes_to_empty() {
    let json = r#"{
        "id": "agg",
        "name": "聚合",
        "strategy": "failover",
        "members": [{"relayId": "relay-a", "weight": 1}]
    }"#;

    let profile: AggregateRelayProfile = serde_json::from_str(json).unwrap();

    assert!(profile.routes.is_empty());
    assert_eq!(profile.members.len(), 1);
}

#[test]
fn aggregate_route_priority_defaults_to_zero_when_missing() {
    let json = r#"{
        "id": "agg",
        "name": "聚合",
        "strategy": "failover",
        "members": [],
        "routes": [{"pattern": "*", "relayId": "relay-a"}]
    }"#;

    let profile: AggregateRelayProfile = serde_json::from_str(json).unwrap();

    assert_eq!(profile.routes[0].priority, 0);
}

#[test]
fn aggregate_profile_with_unknown_fields_deserializes() {
    let json = r#"{
        "id": "agg",
        "name": "聚合",
        "strategy": "failover",
        "members": [],
        "routes": [],
        "futureField": 123
    }"#;

    let profile: AggregateRelayProfile = serde_json::from_str(json).unwrap();

    assert!(profile.routes.is_empty());
}

/// 与标准 glob 语义（递归回溯）全量矩阵对比，防止通配符匹配出现假阴/假阳
#[test]
fn route_wildcard_matches_standard_glob_semantics() {
    fn glob_reference(pattern: &str, model: &str) -> bool {
        let p: Vec<char> = pattern.trim().to_lowercase().chars().collect();
        let m: Vec<char> = model.trim().to_lowercase().chars().collect();
        fn rec(p: &[char], m: &[char]) -> bool {
            match p.split_first() {
                None => m.is_empty(),
                Some(('*', rest)) => (0..=m.len()).any(|take| rec(rest, &m[take..])),
                Some((c, rest)) => m.first() == Some(c) && rec(rest, &m[1..]),
            }
        }
        rec(&p, &m)
    }
    let patterns = [
        "a", "b", "c", "a*", "*a", "a*b", "*a*", "a*b*c", "a*a", "*ab*",
        "ab*cd*ef", "*a*b*a", "**", "*", "a**b", "ab*ab", "*ab*ab*", "a*b*a*c",
        "**a**", "*a*a*a*", "deepseek-*", "*-v3", "gpt-*",
    ];
    let models = [
        "", "a", "b", "ab", "ba", "aa", "aab", "aba", "abb", "abc", "aabb",
        "abab", "abxa", "aabxa", "abcabc", "abcc", "cba", "baab", "ababa",
        "deepseek-chat", "gpt-5.4", "glm-4-v3",
    ];
    let mismatches = patterns
        .iter()
        .flat_map(|pattern| {
            models.iter().filter_map(|model| {
                let actual = match_route_pattern(pattern, model);
                let expected = glob_reference(pattern, model);
                (actual != expected).then(|| (*pattern, *model, expected, actual))
            })
        })
        .collect::<Vec<_>>();
    assert!(mismatches.is_empty(), "mismatches: {mismatches:?}");
}

/// 多字节字符（中文/emoji）与大小写展开（İ -> i̇）不应 panic 或误判
#[test]
fn route_wildcard_handles_unicode_without_panic() {
    assert!(match_route_pattern("deepseek-*", "deepseek-chat中文版"));
    assert!(match_route_pattern("*版", "deepseek-chat中文版"));
    assert!(match_route_pattern("*gpt*", "中文gpt-5"));
    assert!(match_route_pattern("*中*文*", "中文测试"));
    assert!(match_route_pattern("*İ*", "i̇xyz"));
    assert!(match_route_pattern("*i̇*", "xyzİ"));
}

/// 回归：最高优先级路由目标缺 key 时应跳过该规则，命中下一条有效路由
#[test]
fn route_skips_highest_priority_target_missing_key_and_hits_next_valid_route() {
    let mut settings = settings(AggregateRelayStrategy::Failover);
    settings.relay_profiles.push(RelayProfile {
        id: "relay-empty-key".to_string(),
        name: "缺Key成员".to_string(),
        base_url: "https://empty.example.com".to_string(),
        api_key: String::new(),
        ..RelayProfile::default()
    });
    settings.aggregate_relay_profiles[0].members.push(AggregateRelayMember {
        relay_id: "relay-empty-key".to_string(),
        weight: 1,
    });
    settings.aggregate_relay_profiles[0].routes = vec![
        route("deepseek-*", "relay-empty-key", 100),
        route("deepseek-chat", "relay-b", 50),
    ];

    let mut selector = RelayRotationSelector::from_settings(&settings).unwrap();
    let (selected, outcomes) = selector
        .select_with_outcomes(&settings, context(Some("deepseek-chat")))
        .unwrap();

    assert_eq!(selected.id, "relay-b");
    assert_eq!(
        outcomes,
        vec![
            RouteMatchInfo::SkippedInvalidRelay {
                relay_id: "relay-empty-key".to_string(),
                pattern: "deepseek-*".to_string(),
            },
            RouteMatchInfo::Matched {
                relay_id: "relay-b".to_string(),
                pattern: "deepseek-chat".to_string(),
            },
        ]
    );
}

/// 回归：所有路由目标均无效时，回退聚合策略选择有效成员
#[test]
fn route_all_targets_invalid_falls_back_to_aggregate_strategy() {
    let mut settings = settings(AggregateRelayStrategy::Failover);
    settings.relay_profiles.push(RelayProfile {
        id: "relay-empty-key".to_string(),
        name: "缺Key成员".to_string(),
        base_url: "https://empty.example.com".to_string(),
        api_key: String::new(),
        ..RelayProfile::default()
    });
    settings.aggregate_relay_profiles[0].members.push(AggregateRelayMember {
        relay_id: "relay-empty-key".to_string(),
        weight: 1,
    });
    settings.aggregate_relay_profiles[0].routes = vec![
        route("deepseek-*", "relay-empty-key", 100),
        route("deepseek-chat", "relay-empty-key", 50),
    ];

    let mut selector = RelayRotationSelector::from_settings(&settings).unwrap();
    let (selected, outcomes) = selector
        .select_with_outcomes(&settings, context(Some("deepseek-chat")))
        .unwrap();

    // failover 策略选中第一个有效成员 relay-a
    assert_eq!(selected.id, "relay-a");
    assert_eq!(
        outcomes,
        vec![
            RouteMatchInfo::SkippedInvalidRelay {
                relay_id: "relay-empty-key".to_string(),
                pattern: "deepseek-*".to_string(),
            },
            RouteMatchInfo::SkippedInvalidRelay {
                relay_id: "relay-empty-key".to_string(),
                pattern: "deepseek-chat".to_string(),
            },
            RouteMatchInfo::Unmatched,
        ]
    );
}

/// 回归：聚合成员混合有效/无效时，路由命中有效目标后 fallback 列表应过滤掉无效成员，
/// 且只保留仍可用的有效成员作为后续候选
#[test]
fn fallback_after_route_hit_filters_invalid_members() {
    let mut settings = settings(AggregateRelayStrategy::Failover);
    // 在 members 中插入一个无效成员，并让路由命中 relay-b
    settings.aggregate_relay_profiles[0].members.insert(
        1,
        AggregateRelayMember {
            relay_id: "relay-empty-key".to_string(),
            weight: 1,
        },
    );
    settings.relay_profiles.push(RelayProfile {
        id: "relay-empty-key".to_string(),
        name: "缺Key成员".to_string(),
        base_url: "https://empty.example.com".to_string(),
        api_key: String::new(),
        ..RelayProfile::default()
    });
    settings.aggregate_relay_profiles[0].routes = vec![route("deepseek-*", "relay-b", 0)];

    let mut selector = RelayRotationSelector::from_settings(&settings).unwrap();
    let selected = selector
        .select(&settings, context(Some("deepseek-chat")))
        .unwrap();
    assert_eq!(selected.id, "relay-b");

    // fallback_relays_after 应跳过 relay-empty-key，只保留 relay-c、relay-a（按偏移顺序）
    let fallbacks = fallback_relays_after(&settings, &selected.id).unwrap();
    assert_eq!(
        fallbacks
            .iter()
            .map(|profile| profile.id.as_str())
            .collect::<Vec<_>>(),
        vec!["relay-c", "relay-a"]
    );
}

/// 回归：路由命中不应推进轮换状态；即使路由直接选中成员，后续未命中路由时仍应回到当前轮换指针
#[test]
fn route_hit_does_not_advance_failover_rotation_index() {
    let mut settings = settings(AggregateRelayStrategy::Failover);
    settings.aggregate_relay_profiles[0].routes = vec![route("deepseek-*", "relay-b", 0)];
    let mut selector = RelayRotationSelector::from_settings(&settings).unwrap();

    // 第一次请求命中路由 rellay-b
    let first = selector
        .select(&settings, context(Some("deepseek-chat")))
        .unwrap();
    assert_eq!(first.id, "relay-b");
    selector.record_event(RotationEvent::Success);

    // 第二次请求仍是同模型，继续命中路由 rellay-b，failover 指针不应因路由命中而变化
    let second = selector
        .select(&settings, context(Some("deepseek-chat")))
        .unwrap();
    assert_eq!(second.id, "relay-b");

    // 切换到未命中路由的模型，仍应回到 failover 当前指向的 relay-a（索引未被路由推进）
    let fallback = selector
        .select(&settings, context(Some("glm-4")))
        .unwrap();
    assert_eq!(fallback.id, "relay-a");
}

/// 回归：聚合成员列表含无效成员时，未命中路由而策略选中该无效成员应严格报错，
/// 而不是静默跳过到下一个成员（语义与路由阶段跳过无效目标一致但策略成员仍严格校验）
#[test]
fn strategy_selected_invalid_member_returns_strict_error() {
    let mut settings = settings(AggregateRelayStrategy::ConversationRoundRobin);
    settings.relay_profiles.push(RelayProfile {
        id: "relay-empty-key".to_string(),
        name: "缺Key成员".to_string(),
        base_url: "https://empty.example.com".to_string(),
        api_key: String::new(),
        ..RelayProfile::default()
    });
    settings.aggregate_relay_profiles[0].members.insert(
        0,
        AggregateRelayMember {
            relay_id: "relay-empty-key".to_string(),
            weight: 1,
        },
    );
    settings.aggregate_relay_profiles[0].routes = Vec::new();

    let mut selector = RelayRotationSelector::from_settings(&settings).unwrap();
    let error = selector
        .select_with_outcomes(&settings, context(Some("deepseek-chat")))
        .unwrap_err();

    match error {
        SelectionError::InvalidMemberRelay {
            aggregate_id,
            relay_id,
        } => {
            assert_eq!(aggregate_id, "agg");
            assert_eq!(relay_id, "relay-empty-key");
        }
        other => panic!("期望 InvalidMemberRelay，实际得到 {other:?}"),
    }
}
