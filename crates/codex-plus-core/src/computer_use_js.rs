//! Computer Use 的 JS 操作方式：把 `cua_repl` 的 `js` 工具交给客户端。
//!
//! # 为什么有这一档
//!
//! Codex 26.1016 起主要用 `cua_repl`（`unified-computer-use` 插件）操作桌面：模型写一段
//! `cua.getApp(...)` / `app.click(...)` 的 JS，一次调用里可以连做多个动作。`cua_repl`
//! 在第一次使用时会自己下发完整的使用说明与确认规则（来自本机 `@oai/cua/docs`），
//! 并提供 `paste`（中文、多行文字靠它，离散工具的 `type_text` 会静默丢掉键盘打不出的字）。
//! 选这一档，客户端看到的工具、说明和行为都和 Codex 一致。
//!
//! # 和直通档的差别
//!
//! 子进程换成 `cua_repl`，其余沿用直通层（审批、超长帧、断开处理都一样）。这里只补三件事：
//!
//! 1. **回合信息**：`cua_repl` 要求每次 `tools/call` 带
//!    `_meta["x-codex-turn-metadata"] = {session_id, turn_id}`，缺了直接报错。
//!    一个 MCP 进程 = 一个会话；`turn_id` 在回合内稳定。
//! 2. **回合结束**：Claude Code 的 Stop 钩子（`claude_hooks`）触发时发 `turn_ended`，再开新回合。
//!    钩子是本机全局的，本回合没用过 `js` 就不处理，免得替别的会话结束回合。
//!    实测：`turn_ended` 之后同一回合号的调用会被拒，所以发完立即换回合号。
//! 3. **工具面**：只给模型 `js` / `js_reset`。`turn_ended` 由代理自己调，
//!    `js_add_node_module_dir` 用不上；Codex 的插件配置同样只开放这几个。

use std::collections::BTreeSet;

use serde_json::{Value, json};

/// 交给模型的工具。
const VISIBLE_TOOLS: [&str; 2] = ["js", "js_reset"];

/// 代理自己发给 `cua_repl` 的请求 id 前缀（字符串，不会和客户端的数字 id 撞号）。
const OWN_ID_PREFIX: &str = "codexplus-cu-js-";

/// 客户端 → 子进程的一帧要怎么处理。
#[derive(Debug, PartialEq)]
pub(crate) enum ClientFrame {
    /// 原样转发。
    Forward,
    /// 已改写（注入了回合信息），转发改写后的内容。
    Rewritten(Value),
}

/// 子进程 → 客户端的一帧要怎么处理。
#[derive(Debug, PartialEq)]
pub(crate) enum ChildFrame {
    Forward,
    Rewritten(Value),
    /// 代理自己请求的响应，不给客户端。
    Drop,
}

/// JS 档的会话状态。
pub(crate) struct JsBridge {
    session_id: String,
    turn_seq: u64,
    /// 本回合是否调过工具。
    touched_turn: bool,
    /// 客户端发出的 `tools/list` 请求 id（JSON 文本）：响应要过滤工具。
    list_ids: BTreeSet<String>,
    /// 代理自己发出的请求 id：响应不给客户端。
    own_ids: BTreeSet<String>,
    next_own: u64,
}

impl JsBridge {
    pub(crate) fn new(session_id: String) -> Self {
        Self {
            session_id,
            turn_seq: 1,
            touched_turn: false,
            list_ids: BTreeSet::new(),
            own_ids: BTreeSet::new(),
            next_own: 0,
        }
    }

    pub(crate) fn turn_id(&self) -> String {
        format!("{}-{}", self.session_id, self.turn_seq)
    }

    /// 只有 `tools/call` 和 `tools/list` 需要动，其余原样转发。
    pub(crate) fn on_client(&mut self, mut message: Value) -> ClientFrame {
        match message.get("method").and_then(Value::as_str) {
            Some("tools/list") => {
                if let Some(id) = message.get("id") {
                    self.list_ids.insert(id.to_string());
                }
                ClientFrame::Forward
            }
            Some("tools/call") => {
                inject_turn_metadata(&mut message, &self.session_id, &self.turn_id());
                self.touched_turn = true;
                ClientFrame::Rewritten(message)
            }
            _ => ClientFrame::Forward,
        }
    }

    /// 没有待处理的 id 时不必解析（截图帧可能很大）。
    pub(crate) fn wants_child_frames(&self) -> bool {
        !self.list_ids.is_empty() || !self.own_ids.is_empty()
    }

    pub(crate) fn on_child(&mut self, mut message: Value) -> ChildFrame {
        // 通知、子进程发来的请求（如审批）没有对应的待处理 id。
        if message.get("method").is_some() {
            return ChildFrame::Forward;
        }
        let Some(id) = message.get("id").map(Value::to_string) else {
            return ChildFrame::Forward;
        };
        if self.own_ids.remove(&id) {
            return ChildFrame::Drop;
        }
        if self.list_ids.remove(&id) {
            if let Some(tools) = message
                .get_mut("result")
                .and_then(|result| result.get_mut("tools"))
                .and_then(Value::as_array_mut)
            {
                tools.retain(|tool| {
                    tool.get("name")
                        .and_then(Value::as_str)
                        .is_some_and(|name| VISIBLE_TOOLS.contains(&name))
                });
            }
            return ChildFrame::Rewritten(message);
        }
        ChildFrame::Forward
    }

    /// 回合结束：本回合用过工具才发 `turn_ended`（钩子是全局的），发完换新回合号。
    pub(crate) fn end_turn(&mut self) -> Option<Value> {
        if !self.touched_turn {
            return None;
        }
        let turn = self.turn_id();
        self.next_own += 1;
        let id = format!("{OWN_ID_PREFIX}{}", self.next_own);
        self.own_ids.insert(Value::String(id.clone()).to_string());
        let call = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "tools/call",
            "params": {
                "name": "turn_ended",
                "arguments": {
                    "hook_event_name": "Stop",
                    "session_id": self.session_id,
                    "turn_id": turn,
                },
                "_meta": { "x-codex-turn-metadata": turn_metadata(&self.session_id, &turn) }
            }
        });
        self.turn_seq += 1;
        self.touched_turn = false;
        Some(call)
    }
}

fn turn_metadata(session_id: &str, turn_id: &str) -> Value {
    // thread_id 与 session_id 相同：一个 MCP 进程只对应客户端的一个会话。
    json!({ "session_id": session_id, "thread_id": session_id, "turn_id": turn_id })
}

/// 在 `params._meta` 里写入回合信息，`_meta` 里客户端原有的其它键保留。
fn inject_turn_metadata(message: &mut Value, session_id: &str, turn_id: &str) {
    let Some(params) = message.get_mut("params").and_then(Value::as_object_mut) else {
        return;
    };
    let meta = params
        .entry("_meta")
        .or_insert_with(|| Value::Object(serde_json::Map::new()));
    if !meta.is_object() {
        *meta = Value::Object(serde_json::Map::new());
    }
    if let Some(meta) = meta.as_object_mut() {
        meta.insert(
            "x-codex-turn-metadata".into(),
            turn_metadata(session_id, turn_id),
        );
    }
}

/// 自检用的 JS：只读列出应用，结果包在标记里，和 `cua_repl` 首次调用附带的说明分开。
pub const PROBE_CODE: &str = "const apps = await cua.listApps({ emit: false });\n\
nodeRepl.write(\"@@CU-APPS@@\\n\" + apps.map((app) => app.displayName || app.id).join(\"\\n\") + \"\\n@@CU-END@@\");";

/// 从自检输出里取出应用列表（每行一个名字）。
pub fn probe_apps(text: &str) -> Option<&str> {
    let start = text.find("@@CU-APPS@@")? + "@@CU-APPS@@".len();
    let end = text[start..].find("@@CU-END@@")? + start;
    Some(text[start..end].trim_matches('\n'))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bridge() -> JsBridge {
        JsBridge::new("s1".into())
    }

    #[test]
    fn tool_calls_carry_turn_metadata_and_keep_client_meta() {
        let mut bridge = bridge();
        let call = json!({
            "jsonrpc": "2.0", "id": 7, "method": "tools/call",
            "params": { "name": "js", "arguments": { "code": "1" }, "_meta": { "progressToken": 3 } }
        });
        let ClientFrame::Rewritten(out) = bridge.on_client(call) else { panic!("should rewrite") };
        let meta = &out["params"]["_meta"];
        assert_eq!(meta["progressToken"], 3);
        assert_eq!(meta["x-codex-turn-metadata"]["session_id"], "s1");
        assert_eq!(meta["x-codex-turn-metadata"]["thread_id"], "s1");
        assert_eq!(meta["x-codex-turn-metadata"]["turn_id"], "s1-1");
        // 其它请求原样转发。
        assert_eq!(bridge.on_client(json!({ "id": 1, "method": "initialize" })), ClientFrame::Forward);
    }

    #[test]
    fn tools_list_only_exposes_js_and_reset() {
        let mut bridge = bridge();
        assert_eq!(bridge.on_client(json!({ "id": 2, "method": "tools/list" })), ClientFrame::Forward);
        assert!(bridge.wants_child_frames());
        let response = json!({ "id": 2, "result": { "tools": [
            { "name": "js" }, { "name": "js_add_node_module_dir" }, { "name": "js_reset" }, { "name": "turn_ended" }
        ] } });
        let ChildFrame::Rewritten(out) = bridge.on_child(response) else { panic!("should filter") };
        let names: Vec<_> = out["result"]["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap()).collect();
        assert_eq!(names, ["js", "js_reset"]);
        assert!(!bridge.wants_child_frames());
        // 别的响应、通知、子进程发来的请求都原样转发。
        assert_eq!(bridge.on_child(json!({ "id": 3, "result": {} })), ChildFrame::Forward);
        assert_eq!(bridge.on_child(json!({ "id": "x", "method": "elicitation/create" })), ChildFrame::Forward);
    }

    #[test]
    fn turn_end_only_after_use_and_advances_turn() {
        let mut bridge = bridge();
        // 本回合没用过：别的会话的钩子不该结束本会话的回合。
        assert!(bridge.end_turn().is_none());
        assert_eq!(bridge.turn_id(), "s1-1");

        bridge.on_client(json!({ "id": 1, "method": "tools/call", "params": { "name": "js" } }));
        let call = bridge.end_turn().expect("turn_ended");
        assert_eq!(call["params"]["name"], "turn_ended");
        assert_eq!(call["params"]["arguments"]["turn_id"], "s1-1");
        assert_eq!(call["params"]["_meta"]["x-codex-turn-metadata"]["turn_id"], "s1-1");
        assert_eq!(bridge.turn_id(), "s1-2");
        assert!(bridge.end_turn().is_none(), "新回合还没用过");

        // 代理自己请求的响应不给客户端。
        let own_id = call["id"].clone();
        assert_eq!(bridge.on_child(json!({ "id": own_id, "result": {} })), ChildFrame::Drop);
    }

    #[test]
    fn probe_output_is_separated_from_documentation() {
        let text = "## Computer Use\nlots of docs\n@@CU-APPS@@\nFinder\nQQ\n@@CU-END@@\n";
        assert_eq!(probe_apps(text), Some("Finder\nQQ"));
        assert_eq!(probe_apps("no markers"), None);
    }
}
