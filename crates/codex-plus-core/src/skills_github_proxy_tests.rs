use super::build_github_client;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

const CHILD: &str = "skills::github_proxy_tests::route_child";
const KEYS: &[&str] = &[
    "HTTP_PROXY",
    "http_proxy",
    "HTTPS_PROXY",
    "https_proxy",
    "ALL_PROXY",
    "all_proxy",
    "NO_PROXY",
    "no_proxy",
];

fn fixture_key(key: &str) -> &'static str {
    match key {
        "HTTP_PROXY" => "SKILL_FIX_HTTP_UPPER",
        "http_proxy" => "SKILL_FIX_HTTP_LOWER",
        "HTTPS_PROXY" => "SKILL_FIX_HTTPS_UPPER",
        "https_proxy" => "SKILL_FIX_HTTPS_LOWER",
        "ALL_PROXY" => "SKILL_FIX_ALL_UPPER",
        "all_proxy" => "SKILL_FIX_ALL_LOWER",
        "NO_PROXY" => "SKILL_FIX_NO_UPPER",
        "no_proxy" => "SKILL_FIX_NO_LOWER",
        _ => panic!("unexpected environment key"),
    }
}

#[test]
fn route_child() {
    let Ok(target) = std::env::var("SKILL_FIX_TARGET") else {
        return;
    };
    let direct: SocketAddr = std::env::var("SKILL_FIX_DIRECT").unwrap().parse().unwrap();
    let mode = std::env::var("SKILL_FIX_MODE").unwrap();
    let real_env = std::env::var_os("SKILL_FIX_REAL_ENV").is_some();
    let get_env = |key: &str| std::env::var(if real_env { key } else { fixture_key(key) }).ok();
    let has_environment_proxy = [
        "HTTPS_PROXY",
        "https_proxy",
        "ALL_PROXY",
        "all_proxy",
        "HTTP_PROXY",
        "http_proxy",
    ]
    .into_iter()
    .any(|key| get_env(key).is_some_and(|value| !value.trim().is_empty()));
    let system = std::env::var("SKILL_FIX_SYSTEM").ok();
    let builder = reqwest::Client::builder()
        .resolve("api.github.com", direct)
        .resolve("codeload.github.com", direct)
        .resolve("raw.githubusercontent.com", direct);
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let client = match mode.as_str() {
            // Mirror the old Linux system detector: this is a local simulation, not Linux execution.
            "legacy-linux" => {
                let proxy = [
                    "https_proxy",
                    "HTTPS_PROXY",
                    "all_proxy",
                    "ALL_PROXY",
                    "http_proxy",
                    "HTTP_PROXY",
                ]
                .into_iter()
                .find_map(|key| get_env(key).filter(|v| !v.trim().is_empty()))
                .unwrap();
                Ok(builder
                    .proxy(reqwest::Proxy::all(proxy).unwrap())
                    .build()
                    .unwrap())
            }
            "custom-no-proxy" => {
                let proxy = system.unwrap();
                Ok(builder
                    .proxy(
                        reqwest::Proxy::custom(move |_| Some(proxy.clone()))
                            .no_proxy(reqwest::NoProxy::from_string("*")),
                    )
                    .build()
                    .unwrap())
            }
            _ => build_github_client(
                "skill-proxy-test",
                get_env,
                mode == "linux",
                |_| system.clone(),
                builder,
            ),
        };
        if std::env::var_os("SKILL_FIX_BAD_CA").is_some() && has_environment_proxy {
            let error = client.unwrap_err();
            assert!(
                format!("{error:#}").contains("certificate"),
                "unexpected error: {error:#}"
            );
            return;
        }
        let client = client.unwrap();
        if mode == "linux" || mode == "native" {
            assert_eq!(
                format!("{client:?}").contains("TotalTimeout: 90s"),
                !has_environment_proxy
            );
        }
        let request = client.get(format!("https://{target}/"));
        let timeout_test = std::env::var_os("SKILL_FIX_STALL").is_some();
        let result = if timeout_test && has_environment_proxy {
            request.timeout(Duration::from_secs(12))
        } else if timeout_test {
            request
        } else {
            request.timeout(Duration::from_secs(3))
        }
        .send()
        .await;
        if timeout_test {
            assert!(result.unwrap_err().is_timeout());
        } else {
            assert!(
                result.is_err(),
                "the local fixture does not provide a TLS server"
            );
        }
    });
}

struct ManagedChild(Option<Child>);

impl Drop for ManagedChild {
    fn drop(&mut self) {
        if let Some(child) = &mut self.0 {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

fn exercise(
    mode: &str,
    host: &str,
    vars: &[(&str, &str)],
    system: bool,
    real_env: bool,
    bad_ca: bool,
    stall: bool,
    expected: Option<&str>,
) {
    let names = ["direct", "https", "all", "http", "system"];
    let listeners: Vec<_> = names
        .iter()
        .map(|_| {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            listener.set_nonblocking(true).unwrap();
            listener
        })
        .collect();
    let address = |i: usize| listeners[i].local_addr().unwrap();
    let mut command = Command::new(std::env::current_exe().unwrap());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(crate::windows_create_no_window());
    }
    command
        .args(["--exact", CHILD, "--nocapture"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for key in KEYS {
        command.env_remove(key).env_remove(fixture_key(key));
    }
    for key in [
        "CODEX_CA_CERTIFICATE",
        "REQUEST_METHOD",
        "SKILL_FIX_SYSTEM",
        "SKILL_FIX_REAL_ENV",
        "SKILL_FIX_BAD_CA",
        "SKILL_FIX_STALL",
    ] {
        command.env_remove(key);
    }
    command
        .env("SKILL_FIX_MODE", mode)
        .env("SKILL_FIX_DIRECT", address(0).to_string())
        .env("SKILL_FIX_TARGET", format!("{host}:{}", address(0).port()));
    if system {
        command.env("SKILL_FIX_SYSTEM", format!("http://{}", address(4)));
    }
    if real_env {
        command.env("SKILL_FIX_REAL_ENV", "1");
    }
    if bad_ca {
        command.env("SKILL_FIX_BAD_CA", "1").env(
            "CODEX_CA_CERTIFICATE",
            "-----BEGIN CERTIFICATE-----\ninvalid\n-----END CERTIFICATE-----",
        );
    }
    if stall {
        command.env("SKILL_FIX_STALL", "1");
    }
    for (key, value) in vars {
        let value = value
            .replace("@https", &format!("http://{}", address(1)))
            .replace("@all", &format!("http://{}", address(2)))
            .replace("@http", &format!("http://{}", address(3)));
        command.env(
            if real_env || *key == "REQUEST_METHOD" {
                key
            } else {
                fixture_key(key)
            },
            value,
        );
    }
    let mut child = ManagedChild(Some(command.spawn().unwrap()));
    let start = Instant::now();
    let mut observed = Vec::new();
    let mut held = Vec::new();
    loop {
        for (name, listener) in names.iter().zip(&listeners) {
            if let Ok((mut stream, _)) = listener.accept() {
                stream.set_nonblocking(false).unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let mut bytes = [0; 2048];
                let length = stream.read(&mut bytes).unwrap();
                observed.push((*name, bytes[..length].to_vec()));
                if stall {
                    held.push(stream);
                } else if bytes[..length].starts_with(b"CONNECT ") {
                    let _ =
                        stream.write_all(b"HTTP/1.1 502 Bad Gateway\r\nContent-Length: 0\r\n\r\n");
                }
            }
        }
        if child.0.as_mut().unwrap().try_wait().unwrap().is_some() {
            break;
        }
        assert!(
            start.elapsed() < Duration::from_secs(18),
            "fixture timed out: {mode} {host}"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    let output = child.0.take().unwrap().wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{mode} {host} {vars:?}:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    if stall {
        let minimum = if mode == "linux" { 11 } else { 9 };
        assert!(
            start.elapsed() >= Duration::from_secs(minimum),
            "connect timeout returned too early"
        );
    }
    match expected {
        None => assert!(observed.is_empty(), "unexpected route: {observed:?}"),
        Some(expected) => {
            assert_eq!(observed.len(), 1, "{mode} {host} {vars:?}: {observed:?}");
            assert_eq!(
                observed[0].0, expected,
                "{mode} {host} {vars:?}: {observed:?}"
            );
            if expected == "direct" {
                assert!(
                    observed[0].1.starts_with(&[0x16, 0x03]),
                    "expected TLS ClientHello"
                );
            } else {
                assert!(
                    observed[0]
                        .1
                        .starts_with(format!("CONNECT {host}:").as_bytes())
                );
            }
        }
    }
}

#[test]
fn linux_http_only_regression_and_real_environment() {
    for key in ["HTTP_PROXY", "http_proxy"] {
        let vars = [(key, "@http"), ("NO_PROXY", "fixture.invalid")];
        exercise(
            "legacy-linux",
            "api.github.com",
            &vars,
            false,
            true,
            false,
            false,
            Some("http"),
        );
        exercise(
            "linux",
            "api.github.com",
            &vars,
            false,
            true,
            false,
            false,
            Some("direct"),
        );
        exercise(
            "linux",
            "api.github.com",
            &[(key, "@http"), ("REQUEST_METHOD", "GET")],
            false,
            true,
            false,
            false,
            Some("direct"),
        );
    }
}

#[test]
fn linux_http_only_rejects_plaintext_with_credentials() {
    let direct = TcpListener::bind("127.0.0.1:0").unwrap();
    let proxy = TcpListener::bind("127.0.0.1:0").unwrap();
    direct.set_nonblocking(true).unwrap();
    proxy.set_nonblocking(true).unwrap();
    let direct_address = direct.local_addr().unwrap();
    let proxy_address = proxy.local_addr().unwrap();
    let client = build_github_client(
        "skill-proxy-test",
        |key| (key == "HTTP_PROXY").then(|| format!("http://{proxy_address}")),
        true,
        |_| None,
        reqwest::Client::builder().resolve("api.github.com", direct_address),
    )
    .unwrap();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let error = runtime
        .block_on(async {
            client
                .get(format!("http://api.github.com:{}/", direct_address.port()))
                .bearer_auth("fixture-token")
                .send()
                .await
        })
        .unwrap_err();
    assert!(error.is_builder());
    assert_eq!(
        std::error::Error::source(&error)
            .map(ToString::to_string)
            .as_deref(),
        Some("URL scheme is not allowed")
    );
    assert_eq!(
        direct.accept().unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock
    );
    assert_eq!(
        proxy.accept().unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock
    );
}

#[test]
fn proxy_priority_case_and_empty_values() {
    let cases: &[(&str, &[(&str, &str)], bool, &str)] = &[
        (
            "linux",
            &[
                ("HTTPS_PROXY", "@https"),
                ("ALL_PROXY", "@all"),
                ("HTTP_PROXY", "@http"),
            ],
            false,
            "https",
        ),
        (
            "linux",
            &[("https_proxy", "@https"), ("ALL_PROXY", "@all")],
            false,
            "https",
        ),
        (
            "linux",
            &[("https_proxy", "@https"), ("HTTPS_PROXY", "@all")],
            false,
            "https",
        ),
        (
            "linux",
            &[("https_proxy", " "), ("HTTPS_PROXY", "@https")],
            false,
            "https",
        ),
        (
            "linux",
            &[("HTTPS_PROXY", ""), ("all_proxy", "@all")],
            false,
            "all",
        ),
        (
            "linux",
            &[("all_proxy", "@all"), ("ALL_PROXY", "@https")],
            false,
            "all",
        ),
        ("linux", &[("HTTPS_PROXY", "  @https  ")], false, "https"),
        ("linux", &[("HTTP_PROXY", "http://[")], false, "direct"),
        (
            "linux",
            &[
                ("HTTPS_PROXY", " "),
                ("ALL_PROXY", ""),
                ("HTTP_PROXY", "@http"),
            ],
            false,
            "direct",
        ),
        (
            "native",
            &[("HTTPS_PROXY", "@https"), ("https_proxy", "@all")],
            false,
            "https",
        ),
        (
            "native",
            &[("HTTPS_PROXY", "@https"), ("ALL_PROXY", "@all")],
            true,
            "system",
        ),
        ("native", &[("HTTP_PROXY", "@http")], true, "system"),
        ("native", &[], true, "system"),
        ("native", &[], false, "direct"),
    ];
    for (mode, vars, system, expected) in cases {
        exercise(
            mode,
            "api.github.com",
            vars,
            *system,
            false,
            false,
            false,
            Some(expected),
        );
    }
    exercise(
        "linux",
        "api.github.com",
        &[("HTTPS_PROXY", "@https"), ("ALL_PROXY", "@all")],
        false,
        true,
        false,
        false,
        Some("https"),
    );
}

#[test]
fn no_proxy_matches_reqwest_for_each_github_host() {
    let cases = [
        ("api.github.com", "api.github.com", true),
        ("api.github.com", ".github.com", true),
        ("codeload.github.com", " GITHUB.COM , example.org ", true),
        ("raw.githubusercontent.com", "githubusercontent.com", true),
        ("raw.githubusercontent.com", ".github.com", false),
        ("api.github.com", "notgithub.com", false),
        ("api.github.com", "github.com:443", false),
        ("api.github.com", "127.0.0.0/8", false),
        ("api.github.com", "*", true),
        ("api.github.com", "", false),
    ];
    for (host, no_proxy, bypass) in cases {
        let expected = if bypass { "direct" } else { "https" };
        exercise(
            "linux",
            host,
            &[("HTTPS_PROXY", "@https"), ("NO_PROXY", no_proxy)],
            false,
            false,
            false,
            false,
            Some(expected),
        );
        exercise(
            "native",
            host,
            &[("NO_PROXY", no_proxy)],
            true,
            false,
            false,
            false,
            Some("system"),
        );
    }
    for mode in ["linux", "native"] {
        let proxy = if mode == "linux" { "https" } else { "system" };
        let vars = [
            ("HTTPS_PROXY", if mode == "linux" { "@https" } else { "" }),
            ("NO_PROXY", ""),
            ("no_proxy", "github.com"),
        ];
        exercise(
            mode,
            "api.github.com",
            &vars,
            mode == "native",
            false,
            false,
            false,
            Some(proxy),
        );
        let vars = [
            ("HTTPS_PROXY", if mode == "linux" { "@https" } else { "" }),
            ("no_proxy", "github.com"),
        ];
        exercise(
            mode,
            "api.github.com",
            &vars,
            mode == "native",
            false,
            false,
            false,
            Some(if mode == "linux" { "direct" } else { "system" }),
        );
    }
}

#[test]
fn system_proxy_lookups_preserve_upstream_per_host_behavior() {
    let github_urls: &[&str] = &[
        "https://api.github.com/",
        "https://codeload.github.com/",
        "https://raw.githubusercontent.com/",
    ];
    let cases: &[(bool, Option<&str>, &[&str])] = &[
        (false, None, github_urls),
        (false, Some(".github.com"), github_urls),
        (false, Some("*"), github_urls),
        (true, None, &[]),
    ];
    for (linux, no_proxy, expected) in cases {
        let mut lookups = Vec::new();
        let _client = build_github_client(
            "skill-proxy-test",
            |key| {
                if key == "NO_PROXY" {
                    no_proxy.map(str::to_owned)
                } else {
                    None
                }
            },
            *linux,
            |url| {
                lookups.push(url.map(str::to_owned));
                None
            },
            reqwest::Client::builder(),
        )
        .unwrap();
        assert_eq!(
            lookups,
            expected
                .iter()
                .map(|url| Some((*url).to_string()))
                .collect::<Vec<_>>()
        );
    }
}

#[test]
fn no_proxy_ip_and_default_bypass_use_reqwest() {
    for (no_proxy, expected) in [
        ("127.0.0.1", "direct"),
        ("127.0.0.0/8", "direct"),
        ("127.0.1.0/24", "https"),
        ("", "https"),
    ] {
        exercise(
            "linux",
            "127.0.0.1",
            &[("HTTPS_PROXY", "@https"), ("NO_PROXY", no_proxy)],
            false,
            false,
            false,
            false,
            Some(expected),
        );
    }
    exercise(
        "linux",
        "127.0.0.1",
        &[("HTTPS_PROXY", "@https")],
        false,
        false,
        false,
        false,
        Some("direct"),
    );
}

#[test]
fn reqwest_custom_no_proxy_is_not_enforced_in_locked_version() {
    exercise(
        "custom-no-proxy",
        "api.github.com",
        &[],
        true,
        false,
        false,
        false,
        Some("system"),
    );
}

#[test]
fn configured_ca_preserves_previous_proxy_branches() {
    exercise(
        "linux",
        "api.github.com",
        &[("HTTPS_PROXY", "@https")],
        false,
        false,
        true,
        false,
        None,
    );
    exercise(
        "native",
        "api.github.com",
        &[],
        true,
        false,
        true,
        false,
        Some("system"),
    );
}

#[test]
fn stalled_connect_preserves_environment_and_system_timeouts() {
    let environment = std::thread::spawn(|| {
        exercise(
            "linux",
            "api.github.com",
            &[("HTTPS_PROXY", "@https")],
            false,
            false,
            false,
            true,
            Some("https"),
        )
    });
    let system = std::thread::spawn(|| {
        exercise(
            "native",
            "api.github.com",
            &[],
            true,
            false,
            false,
            true,
            Some("system"),
        )
    });
    environment.join().unwrap();
    system.join().unwrap();
}

const TLS_REDIRECT_CHILD: &str = "skills::github_proxy_tests::tls_redirect_client_child";
const TLS_TEST_TOKEN: &str = "skill-proxy-fictional-token";

#[test]
fn tls_redirect_client_child() {
    let Ok(mode) = std::env::var("SKILL_TLS_MODE") else {
        return;
    };
    let port: u16 = std::env::var("SKILL_TLS_PORT").unwrap().parse().unwrap();
    let proxy = std::env::var("SKILL_TLS_PROXY").unwrap();
    let proxy_key = std::env::var("SKILL_TLS_PROXY_KEY").unwrap();
    let cert = reqwest::Certificate::from_pem(include_bytes!(
        "skills_github_proxy_fixtures/test_cert.pem"
    ))
    .unwrap();
    let builder = reqwest::Client::builder()
        .resolve(
            "api.github.com",
            format!("127.0.0.1:{port}").parse().unwrap(),
        )
        .add_root_certificate(cert);
    let client = if mode == "vulnerable-control" {
        builder
            .proxy(reqwest::Proxy::http(&proxy).unwrap())
            .build()
            .unwrap()
    } else if mode == "direct-control" {
        builder.no_proxy().build().unwrap()
    } else {
        // The callback models case-sensitive Linux lookup even on Windows.
        build_github_client(
            "skill-tls-redirect-test",
            |key| match key {
                key if key == proxy_key => Some(proxy.clone()),
                "NO_PROXY" => Some("no-matching-proxy-host.invalid".to_string()),
                _ => None,
            },
            true,
            |_| panic!("Linux HTTP-only must not look up a system proxy"),
            builder,
        )
        .unwrap()
    };

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let result = client
            .get(format!("https://api.github.com:{port}/start"))
            .bearer_auth(TLS_TEST_TOKEN)
            .timeout(Duration::from_secs(5))
            .send()
            .await;
        if mode == "downgrade" {
            let error = result.expect_err("HTTPS-only client followed a downgrade");
            assert!(error.is_redirect(), "unexpected request error: {error:?}");
            let url = error
                .url()
                .expect("redirect error must identify its target");
            assert_eq!(url.scheme(), "http");
            assert_eq!(url.host_str(), Some("api.github.com"));
            assert_eq!(url.port(), Some(port));
            assert_eq!(url.path(), "/final");
        } else {
            let response = result.expect("redirect should succeed");
            assert!(response.status().is_success());
            let expected = if mode == "secure" {
                "secure-final"
            } else {
                "plain-final"
            };
            assert_eq!(response.text().await.unwrap(), expected);
        }
    });
}

fn tls_isolated_command(program: impl AsRef<std::ffi::OsStr>) -> Command {
    let mut command = Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(crate::windows_create_no_window());
    }
    command.env_clear();
    for key in [
        "PATH",
        "SystemRoot",
        "WINDIR",
        "COMSPEC",
        "TEMP",
        "TMP",
        "LD_LIBRARY_PATH",
        "DYLD_LIBRARY_PATH",
    ] {
        if let Some(value) = std::env::var_os(key) {
            command.env(key, value);
        }
    }
    command
}

fn tls_redirect_case(mode: &str, proxy_key: &str, cgi: bool) {
    use std::io::BufRead;
    use std::sync::mpsc;

    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src/skills_github_proxy_fixtures/tls_redirect_server.js");
    let scheme = if mode == "secure" { "https" } else { "http" };
    let mut node = ManagedChild(Some(
        tls_isolated_command("node")
            .arg(fixture)
            .arg(scheme)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .expect("Node.js is required for the ignored TLS redirect tests"),
    ));
    let stdout = node.0.as_mut().unwrap().stdout.take().unwrap();
    let (sender, receiver) = mpsc::channel();
    let reader = std::thread::spawn(move || {
        for line in std::io::BufReader::new(stdout).lines() {
            if sender.send(line).is_err() {
                break;
            }
        }
    });
    let ready = receiver
        .recv_timeout(Duration::from_secs(10))
        .expect("Node TLS fixture did not become ready")
        .expect("could not read Node fixture output");
    let fields: Vec<_> = ready.split_whitespace().collect();
    assert_eq!(fields.len(), 3, "unexpected fixture response: {ready}");
    assert_eq!(fields[0], "READY");
    let port: u16 = fields[1].parse().unwrap();
    let proxy_port: u16 = fields[2].parse().unwrap();
    let proxy = format!("http://127.0.0.1:{proxy_port}");
    let mut command = tls_isolated_command(std::env::current_exe().unwrap());
    command
        .args(["--exact", TLS_REDIRECT_CHILD, "--nocapture"])
        .env("SKILL_TLS_MODE", mode)
        .env("SKILL_TLS_PORT", port.to_string())
        .env("SKILL_TLS_PROXY", &proxy)
        .env("SKILL_TLS_PROXY_KEY", proxy_key)
        .env(proxy_key, &proxy)
        .env("NO_PROXY", "no-matching-proxy-host.invalid")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if cgi {
        command.env("REQUEST_METHOD", "GET");
    }
    let mut client = ManagedChild(Some(command.spawn().unwrap()));
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        if client.0.as_mut().unwrap().try_wait().unwrap().is_some() {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "TLS client child timed out: {mode} {proxy_key} CGI={cgi}"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    let output = client.0.take().unwrap().wait_with_output().unwrap();
    writeln!(node.0.as_mut().unwrap().stdin.as_mut().unwrap(), "report").unwrap();
    let report = receiver
        .recv_timeout(Duration::from_secs(10))
        .expect("Node TLS fixture did not report")
        .expect("could not read Node fixture report");
    let events: serde_json::Value = serde_json::from_str(&report).unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if node.0.as_mut().unwrap().try_wait().unwrap().is_some() {
            break;
        }
        assert!(Instant::now() < deadline, "Node TLS fixture did not exit");
        std::thread::sleep(Duration::from_millis(10));
    }
    let status = node.0.take().unwrap().wait().unwrap();
    reader.join().unwrap();
    assert!(status.success(), "Node TLS fixture failed: {status}");
    assert!(
        output.status.success(),
        "TLS child failed: {mode} {proxy_key} CGI={cgi}; events={events}\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(events["starts"], 1, "{events}");
    assert_eq!(events["initialToken"], true, "{events}");
    assert_eq!(events["redirects"], 1, "{events}");
    assert_eq!(events["proxyConnects"], 0, "{events}");
    if mode == "vulnerable-control" {
        assert_eq!(events["directHttpToken"], false, "{events}");
        assert_eq!(events["proxiedHttp"], 1, "{events}");
        assert_eq!(events["proxiedHttpToken"], true, "{events}");
        assert_eq!(
            events["proxyTarget"],
            format!("http://api.github.com:{port}/final"),
            "{events}"
        );
        assert_eq!(events["directHttp"], 0, "{events}");
    } else if mode == "direct-control" {
        assert_eq!(events["directHttp"], 1, "{events}");
        assert_eq!(events["directHttpToken"], true, "{events}");
        assert_eq!(events["proxiedHttp"], 0, "{events}");
        assert_eq!(events["proxiedHttpToken"], false, "{events}");
    } else {
        assert_eq!(events["directHttpToken"], false, "{events}");
        assert_eq!(events["proxiedHttpToken"], false, "{events}");
        assert_eq!(events["proxiedHttp"], 0, "{events}");
        assert_eq!(events["directHttp"], 0, "{events}");
    }
    if mode == "secure" {
        assert_eq!(events["secureFinals"], 1, "{events}");
        assert_eq!(events["secureFinalToken"], true, "{events}");
    } else {
        assert_eq!(events["secureFinals"], 0, "{events}");
    }
}

fn tls_redirect_matrix(mode: &str) {
    for proxy_key in ["HTTP_PROXY", "http_proxy"] {
        for cgi in [false, true] {
            tls_redirect_case(mode, proxy_key, cgi);
        }
    }
}

// Run with: cargo test --locked --offline -p codex-plus-core --lib tls_redirect_ -- --ignored
#[test]
#[ignore = "requires Node.js on PATH; uses a local test-only TLS certificate"]
fn tls_redirect_blocks_http_downgrade_in_linux_http_only_branch() {
    tls_redirect_matrix("downgrade");
}

#[test]
#[ignore = "requires Node.js on PATH; uses a local test-only TLS certificate"]
fn tls_redirect_allows_https_in_linux_http_only_branch() {
    tls_redirect_matrix("secure");
}

#[test]
#[ignore = "requires Node.js on PATH; uses a local test-only TLS certificate"]
fn tls_redirect_probe_detects_vulnerable_http_proxy_control() {
    tls_redirect_matrix("vulnerable-control");
}

#[test]
#[ignore = "requires Node.js on PATH; uses a local test-only TLS certificate"]
fn tls_redirect_probe_detects_vulnerable_direct_http_control() {
    tls_redirect_matrix("direct-control");
}
