//! 常驻本地协议代理 helper 的生命周期测试。
//!
//! 只用一个测试 + 进程级状态目录覆盖：常驻 helper 是进程级单例，拆成多个
//! 测试并行跑会互相抢端口/状态。

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::time::Duration;

use codex_plus_core::launcher::{DefaultLaunchHooks, LaunchHooks};
use codex_plus_core::resident_helper;

/// 取一个当前空闲的 loopback 端口（先 bind 再释放，测试内立即使用）。
fn free_loopback_port() -> u16 {
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    listener.local_addr().unwrap().port()
}

/// 向 helper 发一个最小 HTTP 请求，返回完整响应文本。
fn post_status(port: u16) -> String {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    stream
        .set_write_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let body = "{}";
    let request = format!(
        "POST /backend/status HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n\
         Content-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(request.as_bytes()).unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    response
}

#[test]
fn resident_helper_stays_up_across_codex_session_lifecycle() {
    // 听写凭据与诊断日志会写进状态目录；测试里整体挪到临时目录，避免影响本机实例。
    let state_dir = tempfile::tempdir().unwrap();
    unsafe {
        std::env::set_var(
            codex_plus_core::paths::STATE_DIR_ENV,
            state_dir.path().to_str().unwrap(),
        );
    }

    let port = free_loopback_port();
    resident_helper::ensure_resident_helper(port).unwrap();
    assert_eq!(resident_helper::resident_helper_port(), Some(port));

    // 常驻 helper 直接可服务，不依赖任何 Codex 会话。
    let response = post_status(port);
    assert!(response.starts_with("HTTP/1.1 200"), "{response}");
    assert!(response.contains("\"status\":\"ok\""), "{response}");
    assert!(
        response.contains("\"transport\":\"http-helper\""),
        "{response}"
    );

    // 幂等：重复 ensure 不会重启或换端口。
    resident_helper::ensure_resident_helper(port).unwrap();
    assert_eq!(resident_helper::resident_helper_port(), Some(port));

    // 模拟一次 Codex++ 启动会话：start_helper 复用常驻实例；会话收尾时
    // shutdown_helper 不能把常驻 helper 或它的听写凭据带走。
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let session_hooks = DefaultLaunchHooks::default();
        session_hooks.start_helper(port).await.unwrap();
        session_hooks.shutdown_helper(port).await;
    });
    assert_eq!(
        resident_helper::resident_helper_port(),
        Some(port),
        "Codex 会话结束不能停止常驻 helper"
    );
    let receipt_path = state_dir.path().join("dictation-helper.json");
    let receipt: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&receipt_path).unwrap()).unwrap();
    assert_eq!(
        receipt["port"],
        serde_json::json!(port),
        "复用常驻 helper 的会话收尾不能删除常驻 helper 的听写凭据"
    );
    assert!(post_status(port).starts_with("HTTP/1.1 200"));

    // 会话 helper 用了另一个端口（例如协议转换暂时关闭、增强 helper 换端口）：
    // 会话收尾后凭据要回到仍在监听的常驻端口，而不是指向已关闭的会话端口。
    let session_port = free_loopback_port();
    runtime.block_on(async {
        let session_hooks = DefaultLaunchHooks::default();
        session_hooks.start_helper(session_port).await.unwrap();
        session_hooks.shutdown_helper(session_port).await;
    });
    assert_eq!(
        resident_helper::resident_helper_port(),
        Some(port),
        "另一个端口的会话 helper 收尾也不能影响常驻实例"
    );
    let receipt: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&receipt_path).unwrap()).unwrap();
    assert_eq!(
        receipt["port"],
        serde_json::json!(port),
        "会话 helper 收尾后听写凭据应回到常驻端口"
    );
    assert!(post_status(port).starts_with("HTTP/1.1 200"));

    // 端口被其它进程占用时 ensure 必须报错而不是假装成功。
    resident_helper::stop_resident_helper();
    assert_eq!(resident_helper::resident_helper_port(), None);
    assert!(!receipt_path.exists(), "常驻 helper 停止后应清理听写凭据");

    let occupied_port = free_loopback_port();
    let occupied = TcpListener::bind(("127.0.0.1", occupied_port)).unwrap();
    assert!(
        resident_helper::ensure_resident_helper(occupied_port).is_err(),
        "端口被占用时应返回错误"
    );
    assert_eq!(resident_helper::resident_helper_port(), None);
    drop(occupied);

    // 占用解除后可以正常再起，确认失败路径没有卡住状态。
    resident_helper::ensure_resident_helper(occupied_port).unwrap();
    assert_eq!(resident_helper::resident_helper_port(), Some(occupied_port));
    resident_helper::stop_resident_helper();
    assert_eq!(resident_helper::resident_helper_port(), None);
    TcpListener::bind(("127.0.0.1", occupied_port)).unwrap();
}
