//! 局域网 / Tailscale 访问用的自签 HTTPS 证书，以及本机可被访问的地址。
//!
//! 证书持久化在状态目录（只给本人读），指纹稳定，手机信任一次即可；
//! 有效期 800 天（iOS 拒绝超过 825 天的服务器证书），快到期或出现证书没覆盖的新地址时重签，
//! 重签时保留旧证书里的地址，免得在家 / 公司两个网络之间来回换证书。

use std::net::{IpAddr, Ipv4Addr, UdpSocket};
use std::path::Path;
use std::sync::Arc;

use anyhow::Context;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const TLS_FILE: &str = "remote-control-tls.json";
const VALID_DAYS: i64 = 800;
const RENEW_BEFORE_DAYS: i64 = 30;
const MAX_SANS: usize = 24;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StoredCertificate {
    pub cert_pem: String,
    pub key_pem: String,
    pub names: Vec<String>,
    /// Unix 秒。
    pub not_after: i64,
}

impl StoredCertificate {
    /// 证书 DER 的 SHA-256，冒号分隔的大写十六进制（与浏览器证书详情里的写法一致）。
    pub fn fingerprint(&self) -> String {
        let der = pem_to_der(&self.cert_pem).unwrap_or_default();
        Sha256::digest(&der).iter().map(|byte| format!("{byte:02X}")).collect::<Vec<_>>().join(":")
    }

    fn covers(&self, names: &[String]) -> bool {
        names.iter().all(|name| self.names.contains(name))
    }
}

fn pem_to_der(pem: &str) -> Option<Vec<u8>> {
    use base64::Engine;
    let body: String = pem.lines().filter(|line| !line.starts_with("-----")).collect();
    base64::engine::general_purpose::STANDARD.decode(body.trim()).ok()
}

/// 与状态文件同目录。
pub fn tls_path() -> std::path::PathBuf {
    super::state_path().with_file_name(TLS_FILE)
}

/// 本机可被手机访问的地址：主网卡的局域网地址、Tailscale 地址（100.64.0.0/10）。
/// 用 UDP connect 问系统路由表选哪个源地址，不会真的发包。
pub fn local_addresses() -> Vec<IpAddr> {
    let mut addresses = Vec::new();
    for probe in ["192.168.0.1:9", "10.0.0.1:9", "8.8.8.8:9", "100.100.100.100:9"] {
        if let Some(ip) = source_address_for(probe) {
            if usable(ip) && !addresses.contains(&ip) {
                addresses.push(ip);
            }
        }
    }
    addresses
}

fn source_address_for(target: &str) -> Option<IpAddr> {
    let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).ok()?;
    socket.connect(target).ok()?;
    Some(socket.local_addr().ok()?.ip())
}

fn usable(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => !v4.is_loopback() && !v4.is_unspecified() && !v4.is_link_local(),
        IpAddr::V6(_) => false,
    }
}

/// Tailscale 的 CGNAT 段。
pub fn is_tailscale(ip: &IpAddr) -> bool {
    matches!(ip, IpAddr::V4(v4) if v4.octets()[0] == 100 && (64..128).contains(&v4.octets()[1]))
}

/// 本机名对应的 mDNS 名（`xxx.local`），取不到就不放。
pub fn local_hostname() -> Option<String> {
    let name = std::env::var("COMPUTERNAME")
        .ok()
        .or_else(|| std::env::var("HOSTNAME").ok())
        .or_else(|| {
            let mut command = std::process::Command::new("hostname");
            #[cfg(windows)]
            {
                use std::os::windows::process::CommandExt;
                command.creation_flags(crate::windows_create_no_window());
            }
            command.output().ok().and_then(|out| String::from_utf8(out.stdout).ok())
        })?;
    let name = name.trim().trim_end_matches(".local").to_ascii_lowercase();
    let valid = !name.is_empty() && name.len() <= 63 && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-');
    valid.then(|| format!("{name}.local"))
}

/// 证书应覆盖的名字：localhost、本机 mDNS 名、当前地址。
pub fn wanted_names(addresses: &[IpAddr]) -> Vec<String> {
    let mut names = vec!["localhost".to_string(), "127.0.0.1".to_string()];
    names.extend(local_hostname());
    names.extend(addresses.iter().map(IpAddr::to_string));
    names
}

pub fn generate(names: &[String], now: time::OffsetDateTime) -> anyhow::Result<StoredCertificate> {
    let mut params = rcgen::CertificateParams::new(names.to_vec()).context("证书地址无效")?;
    params.distinguished_name.push(rcgen::DnType::CommonName, "Codex++ Remote Control");
    params.extended_key_usages = vec![rcgen::ExtendedKeyUsagePurpose::ServerAuth];
    params.not_before = now - time::Duration::days(1);
    params.not_after = now + time::Duration::days(VALID_DAYS);
    let key = rcgen::KeyPair::generate().context("生成证书密钥失败")?;
    let cert = params.self_signed(&key).context("签发证书失败")?;
    Ok(StoredCertificate {
        cert_pem: cert.pem(),
        key_pem: key.serialize_pem(),
        names: names.to_vec(),
        not_after: params.not_after.unix_timestamp(),
    })
}

pub fn load_from(path: &Path) -> Option<StoredCertificate> {
    serde_json::from_slice(&std::fs::read(path).ok()?).ok()
}

/// 读出现有证书；没有、快到期、或没覆盖当前地址时重签（保留旧地址）。
pub fn ensure_certificate_at(path: &Path, addresses: &[IpAddr]) -> anyhow::Result<StoredCertificate> {
    let wanted = wanted_names(addresses);
    let now = time::OffsetDateTime::now_utc();
    let existing = load_from(path);
    if let Some(existing) = &existing {
        let fresh = existing.not_after - now.unix_timestamp() > RENEW_BEFORE_DAYS * 86_400;
        if fresh && existing.covers(&wanted) {
            return Ok(existing.clone());
        }
    }
    let mut names = wanted;
    if let Some(existing) = existing {
        for name in existing.names {
            if !names.contains(&name) && names.len() < MAX_SANS {
                names.push(name);
            }
        }
    }
    let certificate = generate(&names, now)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    crate::settings::atomic_write(path, &serde_json::to_vec_pretty(&certificate)?)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
    Ok(certificate)
}

pub fn server_config(certificate: &StoredCertificate) -> anyhow::Result<Arc<rustls::ServerConfig>> {
    use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
    let cert = CertificateDer::from(pem_to_der(&certificate.cert_pem).context("证书内容无效")?);
    let key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(pem_to_der(&certificate.key_pem).context("证书密钥无效")?));
    let config = rustls::ServerConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
        .with_safe_default_protocol_versions()
        .context("TLS 协议版本配置失败")?
        .with_no_client_auth()
        .with_single_cert(vec![cert], key)
        .context("TLS 证书配置失败")?;
    Ok(Arc::new(config))
}
