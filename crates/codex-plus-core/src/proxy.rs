use std::collections::HashMap;

pub const PROXY_AUTO_DETECT_PORTS: [u16; 5] = [7897, 7890, 10809, 10808, 1080];

pub fn has_proxy_environment(env: &HashMap<String, String>) -> bool {
    [
        "HTTPS_PROXY",
        "HTTP_PROXY",
        "ALL_PROXY",
        "https_proxy",
        "http_proxy",
        "all_proxy",
    ]
    .into_iter()
    .any(|name| env.get(name).is_some_and(|value| !value.is_empty()))
}

pub fn detect_local_proxy() -> Option<String> {
    detect_local_proxy_with(crate::ports::can_connect_loopback_port)
}

pub fn detect_local_proxy_with(can_connect: impl Fn(u16) -> bool) -> Option<String> {
    PROXY_AUTO_DETECT_PORTS
        .into_iter()
        .find(|port| can_connect(*port))
        .map(|port| format!("http://127.0.0.1:{port}"))
}
