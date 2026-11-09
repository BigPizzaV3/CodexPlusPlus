//! 复用 client 按 tokio 运行时分开的回归用例，单独一个测试程序。
//!
//! 它断言「建立了几条连接」，依赖进程级的转发 client 缓存（最多 32 个，键里带运行时 id）。
//! 放在合并的 `tests/all` 里时，上千个并行用例各自起运行时、往同一个缓存里塞 client，
//! 可能在两次请求之间把它的 client 挤掉，于是多建一条连接、计数对不上（CI 上偶发 2≠1、3≠2）。
//! 按 AGENTS.md 的约定，需要独占进程级状态的用例单独留在 `tests/` 下。

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use codex_plus_core::settings::RelayProfile;

/// 复用的 client 按 tokio 运行时分开：连接由建立它的运行时驱动，一个运行时正在结束时
/// 另一个运行时恰好拿到它的连接会报 `dispatch task is gone`。两个运行时同时存活时，
/// 各自只复用自己建的连接，互不共用。
#[test]
fn reused_clients_are_scoped_to_their_runtime() {
    use std::io::{BufRead, BufReader, Read, Write};

    // 保持连接的阻塞式假上游，跑在普通线程里，不依赖任何 tokio 运行时；记建立了几条连接。
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let addr = listener.local_addr().unwrap();
    let accepts = Arc::new(AtomicUsize::new(0));
    let counter = accepts.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(stream) = stream else { return };
            counter.fetch_add(1, Ordering::SeqCst);
            std::thread::spawn(move || {
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut writer = stream;
                loop {
                    let mut length = 0_usize;
                    let mut line = String::new();
                    loop {
                        line.clear();
                        if reader.read_line(&mut line).unwrap_or(0) == 0 {
                            return;
                        }
                        let lower = line.to_ascii_lowercase();
                        if let Some(value) = lower.strip_prefix("content-length:") {
                            length = value.trim().parse().unwrap_or(0);
                        }
                        if line == "\r\n" {
                            break;
                        }
                    }
                    let mut body = vec![0_u8; length];
                    if reader.read_exact(&mut body).is_err() {
                        return;
                    }
                    let response =
                        "HTTP/1.1 200 OK\r\ncontent-type: text/plain\r\ncontent-length: 2\r\n\r\nok";
                    if writer.write_all(response.as_bytes()).is_err() {
                        return;
                    }
                }
            });
        }
    });

    let relay = RelayProfile {
        id: "runtime-scoped".into(),
        base_url: format!("http://{addr}/v1"),
        gateway_proxy: Some("direct".into()),
        ..RelayProfile::default()
    };
    let user_agent = "codex-plus-test/runtime-scoped";
    let url = format!("http://{addr}/v1/ping");
    let runtime = || {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
            .unwrap()
    };
    let fetch = |runtime: &tokio::runtime::Runtime| {
        let (relay, url) = (relay.clone(), url.clone());
        runtime.block_on(async move {
            let client =
                codex_plus_core::http_client::proxied_client_for_relay(user_agent, &relay)
                    .unwrap();
            client.get(url).send().await.unwrap().text().await.unwrap()
        })
    };
    let (first, second) = (runtime(), runtime());
    assert_eq!(fetch(&first), "ok");
    assert_eq!(fetch(&first), "ok");
    assert_eq!(accepts.load(Ordering::SeqCst), 1, "同一运行时连续请求复用连接");
    assert_eq!(fetch(&second), "ok");
    assert_eq!(
        accepts.load(Ordering::SeqCst),
        2,
        "另一个运行时不复用前一个运行时的连接"
    );
    assert_eq!(fetch(&second), "ok");
    assert_eq!(accepts.load(Ordering::SeqCst), 2);
}
