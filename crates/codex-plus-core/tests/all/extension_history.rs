//! 拓展只读访问会话历史：线程表列表、搜索、rollout 整理与分页。

use codex_plus_core::extension_history::{call, route_at};
use serde_json::{Value, json};
use std::fs;

fn home_with_threads() -> tempfile::TempDir {
    let temp = tempfile::tempdir().unwrap();
    let rollout = temp.path().join("rollout-a.jsonl");
    let lines = [
        json!({"type": "session_meta", "payload": {"id": "t-a"}}),
        json!({"type": "turn_context", "payload": {"turn_id": "turn-1"}}),
        json!({"type": "response_item", "timestamp": "2026-10-30T01:00:00Z", "payload": {"type": "message", "role": "developer", "content": [{"type": "input_text", "text": "系统说明"}]}}),
        json!({"type": "response_item", "payload": {"type": "message", "role": "user", "id": "u1", "content": [
            {"type": "input_text", "text": "<environment_context>cwd</environment_context>"},
            {"type": "input_text", "text": "帮我画图"}]}}),
        json!({"type": "response_item", "payload": {"type": "reasoning", "summary": [{"type": "summary_text", "text": "先想想"}], "encrypted_content": "密文"}}),
        json!({"type": "response_item", "payload": {"type": "function_call", "name": "shell", "arguments": "{\"cmd\":\"ls\"}", "call_id": "c1"}}),
        json!({"type": "response_item", "payload": {"type": "function_call_output", "call_id": "c1", "output": "a.txt"}}),
        json!({"type": "response_item", "payload": {"type": "message", "role": "assistant", "id": "m1", "phase": "final_answer", "content": [{"type": "output_text", "text": "画好了"}]}}),
        json!({"type": "event_msg", "payload": {"type": "token_count", "info": {"total_token_usage": {"total_tokens": 120}, "last_token_usage": {"total_tokens": 20}, "model_context_window": 1000}}}),
        json!({"type": "event_msg", "payload": {"type": "task_complete", "turn_id": "turn-1", "duration_ms": 1500, "last_agent_message": "画好了"}}),
        json!({"type": "event_msg", "payload": {"type": "task_started", "turn_id": "turn-2"}}),
        json!({"type": "response_item", "payload": {"type": "message", "role": "user", "id": "u2", "content": [{"type": "input_text", "text": "再改改"}]}}),
        json!({"type": "event_msg", "payload": {"type": "turn_aborted", "turn_id": "turn-2", "reason": "interrupted"}}),
    ];
    fs::write(&rollout, lines.iter().map(Value::to_string).collect::<Vec<_>>().join("\n") + "\nnot json\n").unwrap();
    let db = rusqlite::Connection::open(temp.path().join("state_5.sqlite")).unwrap();
    db.execute_batch(
        "CREATE TABLE threads (id TEXT PRIMARY KEY, rollout_path TEXT, title TEXT, cwd TEXT, model TEXT, created_at INTEGER, updated_at INTEGER, archived INTEGER, first_user_message TEXT);",
    )
    .unwrap();
    for (id, title, updated, archived, rollout) in [
        ("t-a", "画曲线图", 30, 0, rollout.to_string_lossy().to_string()),
        ("t-b", "修 100% 的 bug", 20, 0, String::new()),
        ("t-c", "旧会话", 10, 1, String::new()),
    ] {
        db.execute(
            "INSERT INTO threads VALUES (?1, ?2, ?3, '/work', 'gpt', 1, ?4, ?5, ?3)",
            rusqlite::params![id, rollout, title, updated, archived],
        )
        .unwrap();
    }
    temp
}

fn ids(value: &Value) -> Vec<String> {
    value.as_array().unwrap().iter().map(|session| session["id"].as_str().unwrap().to_string()).collect()
}

#[test]
fn session_queries_list_filter_and_search_read_only() {
    let _guard = crate::common::global_state_lock();
    let home = home_with_threads();
    let home = home.path();
    assert_eq!(ids(&call(home, "listSessions", &json!({})).unwrap()), ["t-a", "t-b"], "新到旧，默认不含归档");
    assert_eq!(ids(&call(home, "listSessions", &json!({"archived": true})).unwrap()), ["t-c"]);
    assert_eq!(ids(&call(home, "filterSessions", &json!({"cwd": "/wo", "createdAfter": 1})).unwrap()), ["t-a", "t-b"]);
    assert_eq!(ids(&call(home, "filterSessions", &json!({"id": "t-b"})).unwrap()), ["t-b"]);
    assert_eq!(ids(&call(home, "filterSessions", &json!({"id": "t-c"})).unwrap()), ["t-c"], "按 id 查不受归档状态限制");
    assert_eq!(ids(&call(home, "filterSessions", &json!({"cwd": "/work", "archived": true})).unwrap()), ["t-c"]);
    assert_eq!(call(home, "readTitle", &json!("t-a")).unwrap(), "画曲线图");

    // % 按字面量匹配，不当通配符；游标分页。
    let found = call(home, "searchSessions", &json!({"query": "100%"})).unwrap();
    assert_eq!(ids(&found["items"]), ["t-b"]);
    assert!(found["nextCursor"].is_null());
    let first = call(home, "searchSessions", &json!({"query": "", "limit": 1})).unwrap();
    assert_eq!(ids(&first["items"]), ["t-a"]);
    let second = call(home, "searchSessions", &json!({"query": "", "limit": 1, "cursor": first["nextCursor"]})).unwrap();
    assert_eq!(ids(&second["items"]), ["t-b"]);

    // 路由包一层 { status, result }；未知方法、缺 id 报错。
    let routed = route_at(home, &json!({"method": "readTitle", "params": {"id": "t-b"}})).unwrap();
    assert_eq!(routed["result"], "修 100% 的 bug");
    assert!(call(home, "deleteSession", &json!("t-a")).unwrap_err().to_string().contains("没有方法"));
    assert!(call(home, "readSession", &json!({})).unwrap_err().to_string().contains("缺少会话 id"));
    assert!(call(home, "readSession", &json!("t-b")).unwrap_err().to_string().contains("没有 rollout"));
}

#[test]
fn session_events_follow_the_event_log_shape() {
    let _guard = crate::common::global_state_lock();
    let home = home_with_threads();
    let home = home.path();
    let read = call(home, "readSession", &json!("t-a")).unwrap();
    assert_eq!(read["session"]["title"], "画曲线图");
    let events = read["events"].as_array().unwrap();
    let kinds = events.iter().map(|event| event["type"].as_str().unwrap()).collect::<Vec<_>>();
    assert_eq!(kinds, [
        "turn/start", "user/message", "assistant/reasoning", "tool/call", "tool/result", "assistant/message",
        "token/usage", "turn/complete",
        "turn/start", "user/message", "turn/aborted",
    ]);
    assert!(events.iter().enumerate().all(|(index, event)| event["seq"] == index), "seq 连续");
    assert_eq!(events[1]["data"]["text"], "帮我画图", "去掉开发者消息与环境上下文");
    assert_eq!(events[1]["data"]["turn"], "turn-1");
    assert_eq!(events[3]["data"]["name"], "shell");
    assert_eq!(events[4]["data"]["output"], "a.txt");
    assert_eq!(events[5]["data"]["phase"], "final_answer");
    assert_eq!(events[6]["data"]["contextWindow"], 1000);
    assert_eq!(events[7]["data"], json!({"turn": "turn-1", "durationMs": 1500, "lastMessage": "画好了"}));
    assert_eq!(events[9]["data"]["turn"], "turn-2");
    assert_eq!(events[10]["data"]["reason"], "interrupted");
    assert!(!read.to_string().contains("密文"), "不返回加密推理");

    let messages = call(home, "listEvents", &json!({"id": "t-a", "types": ["user/message", "assistant/message"], "fromEnd": true, "limit": 2})).unwrap();
    let texts = messages.as_array().unwrap().iter().map(|event| event["data"]["text"].as_str().unwrap()).collect::<Vec<_>>();
    assert_eq!(texts, ["画好了", "再改改"]);
    let filtered = call(home, "filterEvents", &json!({"id": "t-a", "text": "A.TXT"})).unwrap();
    assert_eq!(filtered[0]["type"], "tool/result", "文本检索不分大小写");
    let window = call(home, "readEvent", &json!({"sessionId": "t-a", "seq": 3, "before": 1, "after": 1})).unwrap();
    assert_eq!(window.as_array().unwrap().iter().map(|event| event["seq"].as_u64().unwrap()).collect::<Vec<_>>(), [2, 3, 4]);
    let hits = call(home, "searchEvents", &json!({"query": "再改"})).unwrap();
    assert_eq!(hits["items"][0]["sessionId"], "t-a");
    assert_eq!(hits["items"][0]["event"]["seq"], 9);
}

#[test]
fn cached_events_refresh_when_the_rollout_grows() {
    let _guard = crate::common::global_state_lock();
    let home = home_with_threads();
    let home = home.path();
    let before = call(home, "readEventsSince", &json!({"sessionId": "t-a", "seq": 9})).unwrap();
    assert_eq!(before.as_array().unwrap().len(), 2, "seq 9、10");
    // 往 rollout 追加一条助手消息：缓存按修改时间与大小失效，增量读取能拿到新事件。
    let rollout = home.join("rollout-a.jsonl");
    let mut content = std::fs::read_to_string(&rollout).unwrap();
    content.push_str(&json!({"type": "response_item", "payload": {"type": "message", "role": "assistant", "id": "m2", "content": [{"type": "output_text", "text": "改好了"}]}}).to_string());
    content.push('\n');
    std::fs::write(&rollout, content).unwrap();
    let after = call(home, "readEventsSince", &json!({"sessionId": "t-a", "seq": 11})).unwrap();
    assert_eq!(after[0]["type"], "assistant/message");
    assert_eq!(after[0]["data"]["text"], "改好了");
    assert_eq!(after[0]["seq"], 11);
    let all = call(home, "searchEvents", &json!({"query": "改好"})).unwrap();
    assert_eq!(all["items"][0]["event"]["seq"], 11, "全量搜索看到新内容");
}
