use super::{Index, Snapshot, Tokens, segments, usage_provider};
use std::collections::{HashMap, HashSet};

pub(super) fn resolve(index: &Index, snapshot: &mut Snapshot) -> HashMap<[u8; 32], usize> {
    let mut seen = HashMap::new();
    // 无法确定位置或尚未闭合的缺口仍按整个会话保守处理。
    let incomplete_sessions: HashSet<_> = index
        .files
        .values()
        .filter(|state| state.unlocalized_invalid_counts || state.pending_count_gap)
        .flat_map(|state| {
            std::iter::once(&state.session)
                .chain(state.events.iter().map(|event| &event.session))
                .chain(
                    state
                        .inherited_contexts
                        .iter()
                        .map(|context| &context.session),
                )
        })
        .cloned()
        .collect();
    for event in &mut snapshot.events {
        event.count_incomplete |= incomplete_sessions.contains(&event.session);
    }
    let (streams, contradictory_order) = segments::causal_order(&mut snapshot.events, &index.files);
    snapshot.issues += usize::from(contradictory_order);
    let mut baselines: HashMap<usize, (Tokens, usize)> = HashMap::new();
    let continuation_starts: HashSet<_> = index
        .files
        .values()
        .filter(|state| state.continuation)
        .flat_map(|state| state.events.iter().filter(|event| !event.has_file_baseline))
        .map(|event| event.id)
        .collect();
    let mut session_watermarks: HashMap<String, (Tokens, usize)> = HashMap::new();
    let mut alternate_totals = HashMap::<_, Vec<Tokens>>::new();
    for event in &snapshot.events {
        alternate_totals
            .entry((
                event.session.clone(),
                event.timestamp.clone(),
                event.turn.clone(),
                event.total.input_tokens,
                event.total.output_tokens,
            ))
            .and_modify(|totals| totals.push(event.total))
            .or_insert_with(|| vec![event.total]);
    }
    // 单一证据与已选基线相同，只有竞争证据需要保留并影响本流的下一次增量。
    alternate_totals.retain(|_, totals| totals.len() > 1);
    alternate_totals.shrink_to_fit();
    let mut preceding_totals = HashMap::<usize, &[Tokens]>::new();
    let mut previous_baselines = Vec::with_capacity(snapshot.events.len());
    let mut previous_positions = Vec::with_capacity(snapshot.events.len());
    let mut corrections = Vec::new();
    let component_correction = |before: Tokens, after: Tokens| {
        if before.input_tokens != after.input_tokens || before.output_tokens != after.output_tokens
        {
            return (false, false);
        }
        (
            before.cached_known
                && (!after.cached_known || before.cached_input_tokens != after.cached_input_tokens),
            before.reasoning_output_tokens.is_some()
                && before.reasoning_output_tokens != after.reasoning_output_tokens,
        )
    };
    seen.clear();
    // 模型仅取解析时已有的上下文或精确补证；会话摘要及其他事件不能证明连续性。
    for (position, event) in snapshot.events.iter_mut().enumerate() {
        event.provider =
            usage_provider(&event.model, &event.provider, &index.provider_names).to_owned();
        if event.context_conflicts.provider {
            event.provider.clear();
        }
        let previous = baselines
            .insert(streams[position], (event.total, position))
            .or_else(|| {
                continuation_starts
                    .contains(&event.id)
                    .then(|| session_watermarks.get(&event.session).copied())
                    .flatten()
            });
        session_watermarks.insert(event.session.clone(), (event.total, position));
        previous_positions.push(previous.map(|(_, position)| position));
        let previous = previous.map(|(total, _)| total);
        previous_baselines.push(previous);
        event.count_incomplete |=
            previous.is_none() && !event.has_file_baseline && event.last.is_none();
        if let Some(before) = previous {
            let flags = component_correction(before, event.total);
            if flags.0 || flags.1 {
                corrections.push((previous_positions[position].unwrap(), flags));
            }
        }
        event.count_uncertain |= event.last.is_none()
            && previous.is_some_and(|before| event.total.requires_last_after(before));
        event.delta = event.session_delta(previous);
        // 竞争基线会影响本流下一次增量；其他分段插入不能解除这种不确定性。
        if preceding_totals
            .get(&streams[position])
            .is_some_and(|totals| {
                totals
                    .iter()
                    .any(|total| event.session_delta(Some(*total)) != event.delta)
            })
        {
            event.count_uncertain = true;
        }
        // 缓存/推理分量也可能被补全或修正，不能让它们掩盖相同累计输入输出的竞争记录。
        let evidence_key = (
            event.session.clone(),
            event.timestamp.clone(),
            event.turn.clone(),
            event.total.input_tokens,
            event.total.output_tokens,
        );
        if let Some(totals) = alternate_totals.get(&evidence_key) {
            preceding_totals.insert(streams[position], totals);
            event.count_uncertain = true;
        } else {
            preceding_totals.remove(&streams[position]);
        }
        seen.insert(event.id, position);
    }
    // 使用同一个会话基线比较所有同优先级副本，不能用首次出现的文件决定计数。
    for state in index
        .files
        .values()
        .filter(|state| !state.identity_conflict)
    {
        let mut previous = HashMap::new();
        for candidate in &state.events {
            let Some(&position) = seen.get(&candidate.id) else {
                continue;
            };
            let selected = &mut snapshot.events[position];
            if candidate.inherited != selected.inherited {
                continue;
            }
            if !candidate
                .session_delta(previous_baselines[position])
                .same_counts(selected.delta)
            {
                selected.count_uncertain = true;
            }
            if let Some((before, before_position)) =
                previous.insert(candidate.session.as_str(), (candidate.total, position))
            {
                let flags = component_correction(before, candidate.total);
                if before_position != position && (flags.0 || flags.1) {
                    corrections.push((before_position, flags));
                }
            }
        }
    }
    if !corrections.is_empty() {
        // 沿实际文件前驱和延续基线传播；累计重置及对应分量的修正是边界。
        let mut neighbors = vec![Vec::new(); snapshot.events.len()];
        let mut connect = |before: usize, after: usize| {
            if before == after
                || !snapshot.events[after]
                    .total
                    .is_monotonic_after(snapshot.events[before].total)
            {
                return;
            }
            let boundary =
                component_correction(snapshot.events[before].total, snapshot.events[after].total);
            neighbors[before].push((after, boundary));
            neighbors[after].push((before, boundary));
        };
        for (position, before) in previous_positions.iter().enumerate() {
            if let Some(before) = before {
                connect(*before, position);
            }
        }
        for state in index
            .files
            .values()
            .filter(|state| !state.identity_conflict)
        {
            let mut previous = HashMap::new();
            for candidate in &state.events {
                let Some(&position) = seen.get(&candidate.id) else {
                    continue;
                };
                if candidate.inherited != snapshot.events[position].inherited {
                    continue;
                }
                if let Some(before) = previous.insert(candidate.session.as_str(), position) {
                    connect(before, position);
                }
            }
        }
        let mut affected = vec![(false, false); snapshot.events.len()];
        while let Some((position, (cached, reasoning))) = corrections.pop() {
            let flags = (
                cached && !affected[position].0,
                reasoning && !affected[position].1,
            );
            if !flags.0 && !flags.1 {
                continue;
            }
            affected[position].0 |= flags.0;
            affected[position].1 |= flags.1;
            for &(neighbor, boundary) in &neighbors[position] {
                corrections.push((neighbor, (flags.0 && !boundary.0, flags.1 && !boundary.1)));
            }
        }
        for (event, (cached, reasoning)) in snapshot.events.iter_mut().zip(affected) {
            if cached {
                event.delta.cached_input_tokens = 0;
                event.delta.cached_known = false;
            }
            if reasoning {
                event.delta.reasoning_output_tokens = None;
            }
        }
    }
    seen
}
