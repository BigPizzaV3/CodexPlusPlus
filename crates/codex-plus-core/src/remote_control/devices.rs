//! 远程控制的访问主体：主令牌（管理器 / 本机浏览器）与配对设备。
//!
//! - 主令牌只在本机回环监听上有效，权限是完全控制；
//! - 设备经一次性配对码领取自己的令牌，库里只存令牌的 SHA-256；每台设备有权限等级，可随时吊销；
//! - 配对码只在内存里，5 分钟过期、只能用一次、错 5 次作废。

use std::sync::{Mutex, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, bail};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const PAIRING_TTL: Duration = Duration::from_secs(5 * 60);
const PAIRING_MAX_ATTEMPTS: u32 = 5;
const DEVICE_NAME_MAX_CHARS: usize = 40;
/// 最近活动时间只在内存里随时更新，落盘最多每分钟一次。
const LAST_SEEN_PERSIST_INTERVAL: u64 = 60;

/// 权限等级，由低到高。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Role {
    /// 只能看会话与输出。
    View,
    /// 还能发消息、插话、停止、新建会话、在桌面打开会话。
    Message,
    /// 还能审批命令、回答提问、改模型与权限模式。
    Full,
}

impl Role {
    pub fn parse(text: &str) -> anyhow::Result<Self> {
        match text {
            "view" => Ok(Self::View),
            "message" => Ok(Self::Message),
            "full" => Ok(Self::Full),
            other => bail!("未知的权限等级：{other}"),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::View => "view",
            Self::Message => "message",
            Self::Full => "full",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Device {
    pub id: String,
    pub name: String,
    pub role: Role,
    pub token_sha256: String,
    pub created_at: u64,
    #[serde(default)]
    pub last_seen_at: Option<u64>,
    #[serde(default)]
    pub last_address: Option<String>,
}

impl Device {
    /// 给界面看的样子：不含令牌摘要。
    pub fn public(&self) -> serde_json::Value {
        serde_json::json!({
            "id": self.id,
            "name": self.name,
            "role": self.role.as_str(),
            "createdAt": self.created_at,
            "lastSeenAt": self.last_seen_at,
            "lastAddress": self.last_address,
        })
    }
}

/// 一次请求是谁发的。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Principal {
    Master,
    Device { id: String, name: String, role: Role },
}

impl Principal {
    pub fn role(&self) -> Role {
        match self {
            Self::Master => Role::Full,
            Self::Device { role, .. } => *role,
        }
    }

    pub fn describe(&self) -> serde_json::Value {
        match self {
            Self::Master => serde_json::json!({ "kind": "master", "role": "full", "name": "本机" }),
            Self::Device { id, name, role } => serde_json::json!({ "kind": "device", "id": id, "name": name, "role": role.as_str() }),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Pairing {
    pub code: String,
    pub expires_at: u64,
    pub attempts: u32,
    pub role: Role,
}

#[derive(Default)]
struct Registry {
    devices: Vec<Device>,
    pairing: Option<Pairing>,
    last_persist: u64,
    loaded: bool,
}

fn registry() -> &'static Mutex<Registry> {
    static REGISTRY: OnceLock<Mutex<Registry>> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(Registry::default()))
}

fn lock() -> std::sync::MutexGuard<'static, Registry> {
    let mut guard = registry().lock().unwrap_or_else(|e| e.into_inner());
    if !guard.loaded {
        guard.devices = super::load_state_from(&super::state_path()).devices;
        guard.loaded = true;
    }
    guard
}

fn persist(registry: &mut Registry) -> anyhow::Result<()> {
    let devices = registry.devices.clone();
    super::update_state(|state| state.devices = devices)?;
    registry.last_persist = now();
    Ok(())
}

/// 测试用：丢掉内存里的设备表，下次从状态文件重新读。
pub fn reload() {
    let mut guard = registry().lock().unwrap_or_else(|e| e.into_inner());
    *guard = Registry::default();
}

pub fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs()
}

pub fn token_digest(token: &str) -> String {
    Sha256::digest(token.as_bytes()).iter().map(|byte| format!("{byte:02x}")).collect()
}

fn constant_eq(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// 按令牌找设备，并记下最近活动。
pub fn authorize_device(token: &str, address: Option<&str>) -> Option<Principal> {
    if token.len() != 64 {
        return None;
    }
    let digest = token_digest(token);
    let mut registry = lock();
    let now = now();
    let device = registry.devices.iter_mut().find(|device| constant_eq(&device.token_sha256, &digest))?;
    device.last_seen_at = Some(now);
    if let Some(address) = address {
        device.last_address = Some(address.to_string());
    }
    let principal = Principal::Device { id: device.id.clone(), name: device.name.clone(), role: device.role };
    if now.saturating_sub(registry.last_persist) >= LAST_SEEN_PERSIST_INTERVAL {
        let _ = persist(&mut registry);
    }
    Some(principal)
}

/// 设备是否还在（吊销后已建立的 WebSocket 要断开）。
pub fn device_exists(id: &str) -> bool {
    lock().devices.iter().any(|device| device.id == id)
}

pub fn list() -> Vec<Device> {
    lock().devices.clone()
}

pub fn set_role(id: &str, role: Role) -> anyhow::Result<()> {
    let mut registry = lock();
    let device = registry.devices.iter_mut().find(|device| device.id == id).context("没有这台设备")?;
    device.role = role;
    persist(&mut registry)
}

pub fn revoke(id: &str) -> anyhow::Result<()> {
    let mut registry = lock();
    let before = registry.devices.len();
    registry.devices.retain(|device| device.id != id);
    if registry.devices.len() == before {
        bail!("没有这台设备");
    }
    persist(&mut registry)
}

/// 开始配对：生成新的一次性配对码（替换之前的）。
pub fn start_pairing(role: Role) -> Pairing {
    let code = pairing_code();
    let pairing = Pairing { code, expires_at: now() + PAIRING_TTL.as_secs(), attempts: 0, role };
    lock().pairing = Some(pairing.clone());
    pairing
}

pub fn cancel_pairing() {
    lock().pairing = None;
}

pub fn current_pairing() -> Option<Pairing> {
    let mut registry = lock();
    if registry.pairing.as_ref().is_some_and(|pairing| pairing.expires_at <= now()) {
        registry.pairing = None;
    }
    registry.pairing.clone()
}

/// 用配对码领取设备令牌。成功后配对码作废；错 5 次也作废。
pub fn redeem(code: &str, name: &str) -> anyhow::Result<(Device, String)> {
    let mut registry = lock();
    let Some(pairing) = registry.pairing.as_mut() else { bail!("没有进行中的配对，请在 Codex++ 里重新生成配对码") };
    if pairing.expires_at <= now() {
        registry.pairing = None;
        bail!("配对码已过期，请重新生成");
    }
    if !constant_eq(&pairing.code, &normalize_code(code)) {
        pairing.attempts += 1;
        if pairing.attempts >= PAIRING_MAX_ATTEMPTS {
            registry.pairing = None;
            bail!("配对码错误次数过多，已作废，请重新生成");
        }
        bail!("配对码不对");
    }
    let role = pairing.role;
    registry.pairing = None;
    let token = super::generate_token();
    let name: String = name.trim().chars().filter(|c| !c.is_control()).take(DEVICE_NAME_MAX_CHARS).collect();
    let device = Device {
        id: uuid::Uuid::new_v4().to_string(),
        name: if name.is_empty() { "未命名设备".into() } else { name },
        role,
        token_sha256: token_digest(&token),
        created_at: now(),
        last_seen_at: Some(now()),
        last_address: None,
    };
    registry.devices.push(device.clone());
    persist(&mut registry)?;
    Ok((device, token))
}

/// 8 位配对码，去掉容易看错的字符。
fn pairing_code() -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHJKMNPQRSTUVWXYZ23456789";
    let bytes = uuid::Uuid::new_v4();
    bytes.as_bytes().iter().take(8).map(|byte| ALPHABET[*byte as usize % ALPHABET.len()] as char).collect()
}

pub fn normalize_code(code: &str) -> String {
    code.chars().filter(|c| c.is_ascii_alphanumeric()).map(|c| c.to_ascii_uppercase()).collect()
}
