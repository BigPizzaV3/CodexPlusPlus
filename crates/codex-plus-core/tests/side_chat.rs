use std::fs;
use std::path::Path;

use codex_plus_core::side_chat::{save, source_snapshot};
use serde_json::{Value, json};

const PARENT: &str = "11111111-1111-4111-8111-111111111111";
const SIDE: &str = "22222222-2222-4222-8222-222222222222";
const SAVED: &str = "33333333-3333-4333-8333-333333333333";

fn fixture(home: &Path) -> Value {
    let rows = [
        json!({"type":"session_meta","payload":{"id":PARENT,"session_id":PARENT,"timestamp":"2026-10-21T10:00:00.000Z","cwd":home,"history_mode":"paginated","model_provider":"sidechat_test","originator":"codex_plus_sidechat_test","cli_version":"0.162.0-alpha.2","source":"vscode","base_instructions":{"text":"Synthetic saved conversation test. Never execute historical commands."}}}),
        json!({"type":"event_msg","payload":{"type":"task_started","turn_id":"parent-turn"}}),
        json!({"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"Juniper, November 15"}],"internal_chat_message_metadata_passthrough":{"turn_id":"parent-turn"}}}),
        json!({"type":"event_msg","payload":{"type":"item_completed","thread_id":PARENT,"turn_id":"parent-turn","item":{"type":"UserMessage","id":"parent-user","content":[{"type":"text","text":"Juniper, November 15","text_elements":[]}]}}}),
        json!({"type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"Noted: Juniper, November 15"}],"internal_chat_message_metadata_passthrough":{"turn_id":"parent-turn"}}}),
        json!({"type":"event_msg","payload":{"type":"item_completed","thread_id":PARENT,"turn_id":"parent-turn","item":{"type":"AgentMessage","id":"parent-agent","content":[{"type":"Text","text":"Noted: Juniper, November 15"}]}}}),
        json!({"type":"event_msg","payload":{"type":"task_complete","turn_id":"parent-turn"}}),
    ];
    let content: String = rows
        .iter()
        .enumerate()
        .map(|(ordinal, row)| {
            let mut row = row.clone();
            row["ordinal"] = json!(ordinal);
            row["timestamp"] = json!("2026-10-21T10:00:00.000Z");
            format!("{}\n", serde_json::to_string(&row).unwrap())
        })
        .collect();
    let directory = home.join("sessions");
    fs::create_dir_all(&directory).unwrap();
    fs::write(directory.join(format!("rollout-{PARENT}.jsonl")), &content).unwrap();
    json!({"host_id":"local","source_id":PARENT,"side_id":SIDE,"save_id":SAVED,"source_content":content,"saved_at":"2026-10-21T11:00:00.000Z",
    "turns":[{"turnId":"side-turn","status":"completed","items":[
        {"id":"u1","type":"userMessage","content":[{"type":"text","text":"Use Markdown","text_elements":[]}]},
        {"id":"cmd1","type":"commandExecution","command":"SYNTHETIC_NEVER_EXECUTED","cwd":home,"source":"unifiedExecStartup","status":"failed","aggregatedOutput":"UUID-TEST-ONLY","exitCode":1,"durationMs":532},
        {"id":"a1","type":"agentMessage","text":"Recorded UUID-TEST-ONLY","phase":"final_answer"}
    ]}]})
}

fn saved_rows(result: &Value) -> Vec<Value> {
    fs::read_to_string(result["path"].as_str().unwrap())
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

#[test]
fn saves_parent_and_side_history_without_executing_or_inventing_tool_calls() {
    let home = tempfile::tempdir().unwrap();
    let input = fixture(home.path());
    let result = save(home.path(), &input).unwrap();
    let rows = saved_rows(&result);
    assert_eq!(rows[0]["payload"]["id"], SAVED);
    assert_eq!(rows[0]["payload"]["forked_from_id"], PARENT);
    assert!(
        rows.iter()
            .any(|row| row.to_string().contains("Juniper, November 15"))
    );
    let tool = rows
        .iter()
        .find(|row| row["payload"]["item"]["type"] == "CommandExecution")
        .unwrap();
    assert_eq!(
        tool["payload"]["item"]["aggregated_output"],
        "UUID-TEST-ONLY"
    );
    assert_eq!(tool["payload"]["item"]["status"], "failed");
    assert_eq!(tool["payload"]["item"]["exit_code"], 1);
    assert_eq!(
        tool["payload"]["item"]["duration"],
        json!({"secs":0,"nanos":532_000_000})
    );
    assert!(
        tool["payload"]["item"]["cwd"]
            .as_str()
            .unwrap()
            .starts_with("file:///")
    );
    assert!(
        !rows
            .iter()
            .any(|row| row["payload"]["type"] == "function_call")
    );
    assert!(rows.iter().any(|row| row["type"] == "response_item"
        && row.to_string().contains("UUID-TEST-ONLY")
        && row.to_string().contains("already happened")));
    assert!(
        !home.path().join("state_5.sqlite").exists(),
        "原生恢复前不直接创建索引数据库"
    );
    assert_eq!(
        fs::read_to_string(
            home.path()
                .join("sessions")
                .join(format!("rollout-{PARENT}.jsonl"))
        )
        .unwrap(),
        input["source_content"].as_str().unwrap()
    );
}

#[test]
fn repeated_save_reuses_file_and_conflicting_content_never_overwrites_it() {
    let home = tempfile::tempdir().unwrap();
    let mut input = fixture(home.path());
    let first = save(home.path(), &input).unwrap();
    let original = fs::read(first["path"].as_str().unwrap()).unwrap();
    assert_eq!(save(home.path(), &input).unwrap()["path"], first["path"]);
    input["turns"][0]["items"][2]["text"] = json!("changed after save");
    assert!(save(home.path(), &input).is_err());
    assert_eq!(fs::read(first["path"].as_str().unwrap()).unwrap(), original);
    assert_eq!(
        fs::read_dir(home.path().join("sessions/2026/10/21"))
            .unwrap()
            .count(),
        1
    );
}

#[test]
fn invalid_or_incomplete_data_writes_no_saved_file() {
    let home = tempfile::tempdir().unwrap();
    let input = fixture(home.path());
    for (field, value) in [
        ("host_id", json!("remote")),
        ("save_id", json!(PARENT)),
        ("source_id", json!("../escape")),
    ] {
        let mut invalid = input.clone();
        invalid[field] = value;
        assert!(save(home.path(), &invalid).is_err());
    }
    let mut active = input.clone();
    active["turns"][0]["status"] = json!("inProgress");
    assert!(save(home.path(), &active).is_err());
    let mut images = input.clone();
    images["turns"][0]["items"][0]["content"] =
        json!([{"type":"image","url":"data:image/png;base64,test"}]);
    assert!(
        save(home.path(), &images)
            .unwrap_err()
            .to_string()
            .contains("图片")
    );
    assert!(!home.path().join("sessions/2026").exists());
}

#[test]
fn snapshot_excludes_parent_turns_after_an_explicit_boundary() {
    let home = tempfile::tempdir().unwrap();
    let mut input = fixture(home.path());
    let path = home
        .path()
        .join("sessions")
        .join(format!("rollout-{PARENT}.jsonl"));
    fs::write(&path, format!("{}{}\n",input["source_content"].as_str().unwrap(),json!({"type":"event_msg","payload":{"type":"task_started","turn_id":"later-turn"},"timestamp":"2026-10-21T10:01:00.000Z"}))).unwrap();
    let request = json!({"host_id":"local","source_id":PARENT,"last_turn_id":"parent-turn"});
    let snapshot = source_snapshot(home.path(), &request).unwrap();
    assert!(snapshot.get("content").is_none());
    input.as_object_mut().unwrap().remove("source_content");
    input["source_ref"] = snapshot["source_ref"].clone();
    let rows = saved_rows(&save(home.path(), &input).unwrap());
    assert!(
        !rows
            .iter()
            .any(|row| row.to_string().contains("later-turn"))
    );
    assert!(
        rows.iter()
            .any(|row| row["payload"]["turn_id"] == "parent-turn")
    );
    let before = source_snapshot(
        home.path(),
        &json!({"host_id":"local","source_id":PARENT,"before_turn_id":"parent-turn"}),
    )
    .unwrap();
    input["save_id"] = json!("44444444-4444-4444-8444-444444444444");
    input["source_ref"] = before["source_ref"].clone();
    let rows = saved_rows(&save(home.path(), &input).unwrap());
    assert!(
        !rows
            .iter()
            .any(|row| row.to_string().contains("parent-turn"))
    );
}

#[test]
fn large_local_save_uses_a_fixed_prefix_without_inheriting_the_sharing_limit() {
    use std::io::{Seek, SeekFrom, Write};

    let home = tempfile::tempdir().unwrap();
    let mut input = fixture(home.path());
    let path = home
        .path()
        .join("sessions")
        .join(format!("rollout-{PARENT}.jsonl"));
    let large_text = "x".repeat(17 * 1024 * 1024);
    let mut file = fs::OpenOptions::new().append(true).open(&path).unwrap();
    writeln!(file, "{}", json!({"timestamp":"2026-10-21T10:00:00.000Z","type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":large_text}]}})).unwrap();
    let snapshot = source_snapshot(
        home.path(),
        &json!({"host_id":"local","source_id":PARENT,"check_timestamps":true}),
    )
    .unwrap();
    assert!(snapshot.get("content").is_none());
    assert!(snapshot.to_string().len() < 1024, "主历史不经过页面传输");
    assert_eq!(snapshot["timestamps"], json!(["2026-10-21T10:00:00.000Z"]));
    assert!(
        codex_plus_core::session_share::export_rollout(home.path(), PARENT)
            .unwrap_err()
            .to_string()
            .contains("超过分享大小限制")
    );

    // 创建旁支后，主会话继续追加；保存仍只读取创建时固定的前缀。
    writeln!(
        file,
        "{}",
        json!({"type":"event_msg","payload":{"type":"task_started","turn_id":"later-turn"}})
    )
    .unwrap();
    drop(file);
    input.as_object_mut().unwrap().remove("source_content");
    input["source_ref"] = snapshot["source_ref"].clone();
    let result = save(home.path(), &input).unwrap();
    let rows = saved_rows(&result);
    assert!(
        rows.iter()
            .any(|row| row["payload"]["content"][0]["text"].as_str() == Some(large_text.as_str()))
    );
    assert!(
        !rows
            .iter()
            .any(|row| row["payload"]["turn_id"] == "later-turn")
    );
    drop(rows);
    let saved_path = result["path"].as_str().unwrap();
    let original = fs::read(saved_path).unwrap();

    // 相同长度的改写和截断都拒绝，不把变更后的历史混入已保存会话。
    let mut file = fs::OpenOptions::new().write(true).open(&path).unwrap();
    file.seek(SeekFrom::Start(0)).unwrap();
    file.write_all(b"!").unwrap();
    assert!(
        save(home.path(), &input)
            .unwrap_err()
            .to_string()
            .contains("改写或截断")
    );
    file.set_len(0).unwrap();
    assert!(
        save(home.path(), &input)
            .unwrap_err()
            .to_string()
            .contains("改写或截断")
    );
    assert_eq!(fs::read(saved_path).unwrap(), original);
}

#[test]
fn unfinished_parent_work_is_closed_only_in_the_new_copy() {
    let home = tempfile::tempdir().unwrap();
    let mut input = fixture(home.path());
    let pending = [
        json!({"type":"event_msg","payload":{"type":"task_started","turn_id":"running-parent"}}),
        json!({"type":"response_item","payload":{"type":"function_call","call_id":"pending-call","name":"exec_command","arguments":"{}"}}),
    ];
    let mut content = input["source_content"].as_str().unwrap().to_string();
    for row in pending {
        content.push_str(&format!("{row}\n"));
    }
    input["source_content"] = json!(content);
    let rows = saved_rows(&save(home.path(), &input).unwrap());
    assert!(
        rows.iter()
            .any(|r| r["payload"]["type"] == "function_call_output"
                && r["payload"]["call_id"] == "pending-call")
    );
    assert!(
        rows.iter().any(|r| r["payload"]["type"] == "turn_aborted"
            && r["payload"]["turn_id"] == "running-parent")
    );
}

#[test]
fn unknown_tools_keep_their_entire_record_and_user_text_is_not_id_rewritten() {
    let home = tempfile::tempdir().unwrap();
    let mut input = fixture(home.path());
    input["turns"][0]["items"][0]["content"][0]["text"] = json!(PARENT);
    input["turns"][0]["items"]
        .as_array_mut()
        .unwrap()
        .push(json!({"id":"future1","type":"futureTool","result":{"nested":"preserve this"}}));
    let rows = saved_rows(&save(home.path(), &input).unwrap());
    assert!(rows.iter().any(
        |r| r["payload"]["item"]["id"] == "future1" && r.to_string().contains("preserve this")
    ));
    assert!(
        rows.iter()
            .any(|r| r["payload"]["item"]["type"] == "UserMessage"
                && r["payload"]["item"]["content"][0]["text"] == PARENT)
    );
}

#[test]
fn compacted_parent_calls_do_not_gain_orphaned_results() {
    let home = tempfile::tempdir().unwrap();
    let mut input = fixture(home.path());
    let content = format!(
        "{}{}\n{}\n",
        input["source_content"].as_str().unwrap(),
        json!({"type":"response_item","payload":{"type":"function_call","call_id":"discarded-call","name":"exec_command","arguments":"{}"}}),
        json!({"type":"compacted","payload":{"message":"Synthetic summary","replacement_history":[]}})
    );
    input["source_content"] = json!(content);
    let rows = saved_rows(&save(home.path(), &input).unwrap());
    assert!(
        !rows
            .iter()
            .any(|r| r["payload"]["type"] == "function_call_output"
                && r["payload"]["call_id"] == "discarded-call")
    );
}

// 可选的原生兼容测试：只恢复合成历史，不发送 turn/start，也不使用真实用户目录。
#[test]
#[ignore = "requires CODEX_PLUS_TEST_CODEX_BIN"]
fn native_app_server_restores_saved_history_and_lists_it_after_restart() {
    let binary =
        std::env::var_os("CODEX_PLUS_TEST_CODEX_BIN").expect("set the native Codex executable");
    let home = tempfile::tempdir().unwrap();
    let mut input = fixture(home.path());
    input["turns"][0]["items"].as_array_mut().unwrap().extend([
        json!({"id":"mcp1","type":"mcpToolCall","server":"synthetic","tool":"test","arguments":{"query":"recorded"},"status":"failed","result":null,"error":{"message":"Recorded MCP failure"},"durationMs":120}),
        json!({"id":"r1","type":"reasoning","summary":["Recorded reasoning summary"],"content":[]}),
        json!({"id":"p1","type":"plan","text":"Recorded plan"}),
    ]);
    let saved = save(home.path(), &input).unwrap();
    let log = home.path().join("native-test.log");
    for restarted in [false, true] {
        let mut server = NativeServer::start(&binary, home.path(), &log);
        server.request("initialize", json!({"clientInfo":{"name":"codex_plus_sidechat_test","version":"0.1.0"},"capabilities":{"experimentalApi":true}}));
        server.send(json!({"method":"initialized","params":{}}));
        let mut params = json!({"threadId":SAVED,"excludeTurns":false});
        if !restarted {
            params["path"] = saved["path"].clone();
        }
        server.request("thread/resume", params);
        let read = server.request("thread/read", json!({"threadId":SAVED,"includeTurns":true}));
        let thread = &read["thread"];
        assert_eq!(thread["ephemeral"], false);
        let turns = thread["turns"].as_array().unwrap();
        assert_eq!(turns.len(), 2, "{read}");
        assert_eq!(turns[0]["id"], "parent-turn");
        assert_eq!(
            turns[0]["items"][0]["content"][0]["text"],
            "Juniper, November 15"
        );
        assert_eq!(turns[1]["id"], "side-turn");
        assert_eq!(turns[1]["status"], "completed");
        assert_eq!(turns[1]["items"][0]["content"][0]["text"], "Use Markdown");
        assert_eq!(turns[1]["items"][1]["aggregatedOutput"], "UUID-TEST-ONLY");
        assert_eq!(turns[1]["items"][1]["status"], "failed");
        assert_eq!(turns[1]["items"][1]["exitCode"], 1);
        assert_eq!(turns[1]["items"][2]["text"], "Recorded UUID-TEST-ONLY");
        assert_eq!(turns[1]["items"][3]["type"], "mcpToolCall");
        assert_eq!(
            turns[1]["items"][3]["error"]["message"],
            "Recorded MCP failure"
        );
        assert_eq!(
            turns[1]["items"][4]["summary"][0],
            "Recorded reasoning summary"
        );
        assert_eq!(turns[1]["items"][5]["text"], "Recorded plan");
        let listed = server.request(
            "thread/list",
            json!({"limit":20,"sourceKinds":["appServer","cli","vscode","exec","unknown"]}),
        );
        assert!(
            listed["data"]
                .as_array()
                .unwrap()
                .iter()
                .any(|t| t["id"] == SAVED),
            "listed={listed}, restored={read}"
        );
        drop(server);
        let diagnostics = fs::read_to_string(&log).unwrap();
        assert!(
            !diagnostics.contains("failed to parse rollout"),
            "{diagnostics}"
        );
    }
}

struct NativeServer {
    child: std::process::Child,
    stdin: std::process::ChildStdin,
    messages: std::sync::mpsc::Receiver<Value>,
    next_id: u64,
}

impl NativeServer {
    fn start(binary: &std::ffi::OsStr, home: &Path, log: &Path) -> Self {
        use std::io::BufRead;
        use std::process::{Command, Stdio};
        let mut command = Command::new(binary);
        command.args(["app-server", "--listen", "stdio://"]);
        for setting in [
            "model=\"sidechat-test-model\"",
            "model_provider=\"sidechat_test\"",
            "model_providers.sidechat_test.name=\"Isolated test\"",
            "model_providers.sidechat_test.base_url=\"http://127.0.0.1:1/v1\"",
            "model_providers.sidechat_test.wire_api=\"responses\"",
            "approval_policy=\"never\"",
            "sandbox_mode=\"read-only\"",
            "features.shell_snapshot=false",
        ] {
            command.args(["-c", setting]);
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x08000000);
        }
        let mut child = command
            .env("CODEX_HOME", home)
            .env("CODEX_SQLITE_HOME", home)
            .current_dir(home)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(fs::File::create(log).unwrap())
            .spawn()
            .unwrap();
        let stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let (tx, messages) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            for line in std::io::BufReader::new(stdout)
                .lines()
                .map_while(Result::ok)
            {
                if let Ok(value) = serde_json::from_str(&line) {
                    if tx.send(value).is_err() {
                        break;
                    }
                }
            }
        });
        Self {
            child,
            stdin,
            messages,
            next_id: 0,
        }
    }

    fn send(&mut self, value: Value) {
        use std::io::Write;
        writeln!(self.stdin, "{value}").unwrap();
        self.stdin.flush().unwrap();
    }

    fn request(&mut self, method: &str, params: Value) -> Value {
        assert_ne!(
            method, "turn/start",
            "this test must never execute a model or tool"
        );
        self.next_id += 1;
        self.send(json!({"id":self.next_id,"method":method,"params":params}));
        loop {
            let response = self
                .messages
                .recv_timeout(std::time::Duration::from_secs(20))
                .unwrap();
            if response["id"] == self.next_id {
                assert!(response.get("error").is_none(), "{method}: {response}");
                return response["result"].clone();
            }
        }
    }
}

impl Drop for NativeServer {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
