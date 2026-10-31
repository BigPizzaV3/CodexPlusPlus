use std::ffi::{OsStr, OsString};
use std::fs;
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{Receiver, channel};
use std::time::{Duration, Instant};

use codex_plus_core::relay_config::ensure_managed_session_provider_in_home;
use codex_plus_core::settings::BackendSettings;
use serde_json::{Value, json};

const THREAD_ID: &str = "77777777-7777-4777-8777-777777777777";
const TURN_ID: &str = "managed-history-turn";
const USER_TEXT: &str = "Synthetic historical question";
const ASSISTANT_TEXT: &str = "Synthetic recorded answer";

#[test]
#[ignore = "requires CODEX_PLUS_TEST_CODEX_BIN"]
fn native_official_resume_requires_managed_alias_and_preserves_history_after_restart() {
    let _state = crate::common::global_state_lock();
    let binary = std::env::var_os("CODEX_PLUS_TEST_CODEX_BIN")
        .expect("set CODEX_PLUS_TEST_CODEX_BIN to the native Codex executable");
    assert!(
        Path::new(&binary).is_file(),
        "native Codex executable is missing"
    );
    let home = tempfile::tempdir().unwrap();
    let rollout = fixture(home.path());
    let original = fs::read(&rollout).unwrap();
    let params = json!({"threadId":THREAD_ID,"path":rollout,"excludeTurns":false});

    {
        let mut server = NativeServer::start(&binary, home.path(), "before-repair");
        let response = server.response("thread/resume", params.clone());
        let error = response["error"]["message"]
            .as_str()
            .expect("native resume must fail before repairing the missing provider");
        let error = error.to_ascii_lowercase();
        assert!(
            error.contains("model provider")
                && error.contains("codex_plus_managed")
                && error.contains("not found"),
            "unexpected native resume error: {response}"
        );
    }

    let index = home.path().join("repair-index");
    fs::create_dir_all(&index).unwrap();
    let connection = rusqlite::Connection::open(index.join("state_5.sqlite")).unwrap();
    connection
        .execute_batch("CREATE TABLE threads (model_provider TEXT); INSERT INTO threads VALUES ('codex_plus_managed');")
        .unwrap();
    drop(connection);
    {
        let _sqlite_home = SqliteHomeGuard::set(&index);
        assert!(
            ensure_managed_session_provider_in_home(home.path(), &BackendSettings::default())
                .unwrap()
        );
    }
    let config = fs::read_to_string(home.path().join("config.toml")).unwrap();
    let doc = config.parse::<toml_edit::DocumentMut>().unwrap();
    assert_eq!(doc["model_provider"].as_str(), Some("openai"));
    let provider = &doc["model_providers"]["codex_plus_managed"];
    assert_eq!(provider["requires_openai_auth"].as_bool(), Some(true));
    assert!(provider.get("base_url").is_none());
    assert!(provider.get("experimental_bearer_token").is_none());

    for phase in ["after-repair", "after-restart"] {
        let mut server = NativeServer::start(&binary, home.path(), phase);
        let resume_params = if phase == "after-repair" {
            params.clone()
        } else {
            json!({"threadId":THREAD_ID,"excludeTurns":false})
        };
        let resumed = server.request("thread/resume", resume_params);
        assert_eq!(resumed["thread"]["id"], THREAD_ID);
        let read = server.request(
            "thread/read",
            json!({"threadId":THREAD_ID,"includeTurns":true}),
        );
        assert_eq!(read["thread"]["id"], THREAD_ID);
        let turns = read["thread"]["turns"].as_array().unwrap();
        assert_eq!(turns.len(), 1, "{phase}: {read}");
        assert_eq!(turns[0]["id"], TURN_ID);
        assert_eq!(turns[0]["status"], "completed");
        let items = turns[0]["items"].as_array().unwrap();
        assert_eq!(items.len(), 2, "{phase}: {read}");
        assert_eq!(items[0]["type"], "userMessage");
        assert_eq!(items[0]["content"][0]["text"], USER_TEXT);
        assert_eq!(items[1]["type"], "agentMessage");
        assert_eq!(items[1]["text"], ASSISTANT_TEXT);
        drop(server);
        assert!(!home.path().join("auth.json").exists());
    }
    let persisted = fs::read_to_string(&rollout).unwrap();
    let metadata: Value =
        serde_json::from_slice(original.split(|byte| *byte == b'\n').next().unwrap()).unwrap();
    let restored_metadata: Value = serde_json::from_str(persisted.lines().next().unwrap()).unwrap();
    assert_eq!(restored_metadata, metadata);
    assert!(!home.path().join("auth.json").exists());
}

fn fixture(home: &Path) -> PathBuf {
    fs::write(
        home.join("config.toml"),
        "model = 'gpt-5'\nmodel_provider = 'openai'\ncli_auth_credentials_store = 'file'\nmcp_oauth_credentials_store = 'file'\napproval_policy = 'never'\nsandbox_mode = 'read-only'\n[features]\nshell_snapshot = false\n",
    )
    .unwrap();
    let rows = [
        json!({"type":"session_meta","payload":{"id":THREAD_ID,"session_id":THREAD_ID,"timestamp":"2026-10-30T00:00:00.000Z","cwd":home,"history_mode":"paginated","model_provider":"codex_plus_managed","originator":"codex_plus_managed_resume_test","cli_version":"0.162.0-alpha.17.2","source":"vscode","base_instructions":{"text":"Synthetic history only."}}}),
        json!({"type":"event_msg","payload":{"type":"task_started","turn_id":TURN_ID}}),
        json!({"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":USER_TEXT}],"internal_chat_message_metadata_passthrough":{"turn_id":TURN_ID}}}),
        json!({"type":"event_msg","payload":{"type":"item_completed","thread_id":THREAD_ID,"turn_id":TURN_ID,"item":{"type":"UserMessage","id":"managed-user","content":[{"type":"text","text":USER_TEXT,"text_elements":[]}]}}}),
        json!({"type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":ASSISTANT_TEXT}],"internal_chat_message_metadata_passthrough":{"turn_id":TURN_ID}}}),
        json!({"type":"event_msg","payload":{"type":"item_completed","thread_id":THREAD_ID,"turn_id":TURN_ID,"item":{"type":"AgentMessage","id":"managed-agent","content":[{"type":"Text","text":ASSISTANT_TEXT}]}}}),
        json!({"type":"event_msg","payload":{"type":"task_complete","turn_id":TURN_ID}}),
    ];
    let content: String = rows
        .into_iter()
        .enumerate()
        .map(|(ordinal, mut row)| {
            row["ordinal"] = json!(ordinal);
            row["timestamp"] = json!("2026-10-30T00:00:00.000Z");
            format!("{row}\n")
        })
        .collect();
    let sessions = home.join("sessions/2026/10/30");
    fs::create_dir_all(&sessions).unwrap();
    let rollout = sessions.join(format!("rollout-2026-10-30T00-00-00-{THREAD_ID}.jsonl"));
    fs::write(&rollout, content).unwrap();
    rollout
}

struct SqliteHomeGuard(Option<OsString>);

impl SqliteHomeGuard {
    fn set(home: &Path) -> Self {
        let previous = std::env::var_os("CODEX_SQLITE_HOME");
        unsafe { std::env::set_var("CODEX_SQLITE_HOME", home) };
        Self(previous)
    }
}

impl Drop for SqliteHomeGuard {
    fn drop(&mut self) {
        unsafe {
            match &self.0 {
                Some(previous) => std::env::set_var("CODEX_SQLITE_HOME", previous),
                None => std::env::remove_var("CODEX_SQLITE_HOME"),
            }
        }
    }
}

struct NativeServer {
    child: Child,
    stdin: Option<ChildStdin>,
    messages: Receiver<Value>,
    reader: Option<std::thread::JoinHandle<()>>,
    next_id: u64,
    log: PathBuf,
    #[cfg(windows)]
    job: WindowsJob,
}

impl NativeServer {
    fn start(binary: &OsStr, home: &Path, phase: &str) -> Self {
        #[cfg(windows)]
        use std::os::windows::process::CommandExt;

        let mut command = Command::new(binary);
        command
            .args(["app-server", "--listen", "stdio://"])
            .env_clear();
        for directory in ["appdata", "localappdata", "user", "tmp", "sqlite", "xdg"] {
            fs::create_dir_all(home.join(directory)).unwrap();
        }
        for (key, directory) in [
            ("APPDATA", "appdata"),
            ("LOCALAPPDATA", "localappdata"),
            ("HOME", "user"),
            ("USERPROFILE", "user"),
            ("TEMP", "tmp"),
            ("TMP", "tmp"),
            ("TMPDIR", "tmp"),
            ("XDG_CONFIG_HOME", "xdg"),
            ("XDG_DATA_HOME", "xdg"),
            ("XDG_CACHE_HOME", "xdg"),
            ("CODEX_SQLITE_HOME", "sqlite"),
        ] {
            command.env(key, home.join(directory));
        }
        command.env("CODEX_HOME", home);
        #[cfg(windows)]
        {
            let system_root = std::env::var_os("SystemRoot").expect("SystemRoot is required");
            command
                .env("PATH", Path::new(&system_root).join("System32"))
                .env("WINDIR", &system_root)
                .env("SystemRoot", system_root)
                .creation_flags(0x08000000);
        }
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            command.env("PATH", "/usr/bin:/bin").process_group(0);
        }
        let log = home.join(format!("{phase}.log"));
        let mut child = command
            .current_dir(home)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(fs::File::create(&log).unwrap())
            .spawn()
            .unwrap();
        #[cfg(windows)]
        let job = match WindowsJob::attach(&child) {
            Ok(job) => job,
            Err(error) => {
                let _ = Command::new(
                    Path::new(&std::env::var_os("SystemRoot").unwrap())
                        .join("System32/taskkill.exe"),
                )
                .args(["/PID", &child.id().to_string(), "/T", "/F"])
                .creation_flags(0x08000000)
                .output();
                let _ = child.kill();
                let _ = child.wait();
                panic!("cannot isolate native app-server process tree: {error}");
            }
        };
        let stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let (sender, messages) = channel();
        let reader = std::thread::spawn(move || {
            for line in std::io::BufReader::new(stdout)
                .lines()
                .map_while(Result::ok)
            {
                if let Ok(message) = serde_json::from_str::<Value>(&line) {
                    if sender.send(message).is_err() {
                        break;
                    }
                }
            }
        });
        let mut server = Self {
            child,
            stdin: Some(stdin),
            messages,
            reader: Some(reader),
            next_id: 0,
            log,
            #[cfg(windows)]
            job,
        };
        server.request("initialize", json!({"clientInfo":{"name":"codex_plus_managed_resume_test","version":"0.1.0"},"capabilities":{"experimentalApi":true}}));
        server.send(json!({"method":"initialized","params":{}}));
        server
    }

    fn send(&mut self, value: Value) {
        let stdin = self.stdin.as_mut().unwrap();
        writeln!(stdin, "{value}").unwrap();
        stdin.flush().unwrap();
    }

    fn response(&mut self, method: &str, params: Value) -> Value {
        assert!(matches!(
            method,
            "initialize" | "thread/resume" | "thread/read"
        ));
        self.next_id += 1;
        self.send(json!({"id":self.next_id,"method":method,"params":params}));
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            let response = self
                .messages
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .unwrap_or_else(|error| {
                    panic!(
                        "{method}: {error}; {}",
                        fs::read_to_string(&self.log).unwrap_or_default()
                    )
                });
            if response["id"].as_u64() == Some(self.next_id) {
                return response;
            }
        }
    }

    fn request(&mut self, method: &str, params: Value) -> Value {
        let response = self.response(method, params);
        assert!(response.get("error").is_none(), "{method}: {response}");
        response["result"].clone()
    }
}

impl Drop for NativeServer {
    fn drop(&mut self) {
        #[cfg(windows)]
        self.job.terminate();
        #[cfg(unix)]
        {
            unsafe extern "C" {
                fn kill(pid: i32, signal: i32) -> i32;
            }
            unsafe { kill(-i32::try_from(self.child.id()).unwrap(), 9) };
        }
        self.stdin.take();
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }
}

#[cfg(windows)]
struct WindowsJob(*mut std::ffi::c_void);

#[cfg(windows)]
#[link(name = "kernel32")]
unsafe extern "system" {
    fn CreateJobObjectW(
        attributes: *const std::ffi::c_void,
        name: *const u16,
    ) -> *mut std::ffi::c_void;
    fn AssignProcessToJobObject(job: *mut std::ffi::c_void, process: *mut std::ffi::c_void) -> i32;
    fn TerminateJobObject(job: *mut std::ffi::c_void, exit_code: u32) -> i32;
    fn CloseHandle(handle: *mut std::ffi::c_void) -> i32;
}

#[cfg(windows)]
impl WindowsJob {
    fn attach(child: &Child) -> std::io::Result<Self> {
        use std::os::windows::io::AsRawHandle;
        let handle = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
        if handle.is_null() {
            return Err(std::io::Error::last_os_error());
        }
        let job = Self(handle);
        if unsafe { AssignProcessToJobObject(job.0, child.as_raw_handle()) } == 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(job)
    }

    fn terminate(&self) {
        if unsafe { TerminateJobObject(self.0, 1) } == 0 {
            eprintln!(
                "cannot terminate owned native app-server job: {}",
                std::io::Error::last_os_error()
            );
        }
    }
}

#[cfg(windows)]
impl Drop for WindowsJob {
    fn drop(&mut self) {
        self.terminate();
        unsafe { CloseHandle(self.0) };
    }
}
