use std::net::{TcpListener, ToSocketAddrs};

pub const LAUNCHER_GUARD_PORT: u16 = 57320;
pub const MANAGER_GUARD_PORT: u16 = 57319;

pub fn select_platform_loopback_port(requested: u16) -> u16 {
    select_platform_loopback_port_with(
        requested,
        cfg!(windows),
        can_bind_loopback_port,
        find_available_loopback_port,
    )
}

pub fn select_platform_loopback_port_with(
    requested: u16,
    is_windows: bool,
    can_bind: impl Fn(u16) -> bool,
    find_available: impl Fn() -> u16,
) -> u16 {
    if !is_windows || can_bind(requested) {
        requested
    } else {
        find_available()
    }
}

pub fn can_bind_loopback_port(port: u16) -> bool {
    if port == 0 {
        return true;
    }
    TcpListener::bind(("127.0.0.1", port)).is_ok()
}

pub fn find_available_loopback_port() -> u16 {
    TcpListener::bind(("127.0.0.1", 0))
        .and_then(|listener| listener.local_addr())
        .map(|address| address.port())
        .unwrap_or(0)
}

pub fn can_connect_loopback_port(port: u16) -> bool {
    ("127.0.0.1", port)
        .to_socket_addrs()
        .ok()
        .and_then(|mut addresses| addresses.next())
        .and_then(|address| {
            std::net::TcpStream::connect_timeout(&address, std::time::Duration::from_millis(200))
                .ok()
        })
        .is_some()
}

pub fn acquire_loopback_port_guard(port: u16) -> std::io::Result<TcpListener> {
    TcpListener::bind(("127.0.0.1", port))
}

pub fn acquire_resilient_loopback_port_guard(
    port: u16,
) -> std::io::Result<(TcpListener, Option<u16>)> {
    match acquire_loopback_port_guard(port) {
        Ok(listener) => Ok((listener, None)),
        Err(error) if error.kind() == std::io::ErrorKind::AddrInUse => {
            if can_connect_loopback_port(port) {
                Err(error)
            } else {
                let listener = TcpListener::bind(("127.0.0.1", 0))?;
                let actual_port = listener.local_addr().ok().map(|address| address.port());
                Ok((listener, actual_port))
            }
        }
        Err(error) => Err(error),
    }
}
