//! Qoder（qoder.com）与 Qoder CN（qoder.cn）：两站协议相同，只是域名和 client id 不同，
//! 账号不互通（qoder.cn 的账号不能在 qoder.com 登录）。
//!
//! - 登录：浏览器设备流（PKCE：打开授权页，轮询 device token），再用 device token 换聊天用的
//!   job token；也可以粘贴个人访问令牌（PAT）换 job token。refresh token 只能用一次，新的必须写回。
//! - 模型列表与聊天走 COSY 签名：账号信息用一次性 AES-128-CBC 密钥加密、密钥用 Qoder 的 RSA
//!   公钥包起来，整个请求再做一次 MD5 签名。聊天请求体用打乱字母表的 base64 编码。
//! - 聊天要先拉模型列表：请求里的 `model_config` 必须是列表里那一项的原文。
//! - 答复是 SSE，每行一个信封 `{statusCodeValue, body}`，`body` 是 OpenAI 的分片 JSON 字符串；
//!   有的模型把工具调用写在正文里（`<tool_call>` / `<function=…>` / `<invoke>`），要切出来。

use std::collections::{HashMap, HashSet};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use anyhow::Context;
use base64::Engine;
use serde_json::{Value, json};

use crate::gateway::subscriptions::b_shared::{
    self, ChatRelay, Sse, SseHandler, content_text, decode_stream, sha256_hex, system_texts,
    uuid_from_seed,
};
use crate::gateway::subscriptions::oauth;
use crate::gateway::subscriptions::{
    ChatStream, LoginDone, LoginMethod, LoginStart, Subscription, fail,
};

/// 必须是桌面 app 的版本（`Cosy-ClientType` 声明的是桌面端）。
const COSY_VERSION: &str = "1.1.69";
const RSA_KEY: &str = "MIGfMA0GCSqGSIb3DQEBAQUAA4GNADCBiQKBgQDA8iMH5c02LilrsERw9t6Pv5Nc4k6Pz1EaDicBMpdpxKduSZu5OANqUq8er4GM95omAGIOPOh+Nx0spthYA2BqGz+l6HRkPJ7S236FZz73In/KVuLnwI8JJ2CbuJap8kvheCCZpmAWpb/cPx/3Vr/J6I17XcW+ML9FoCI6AOvOzwIDAQAB";
const ALPHABET: &[u8; 64] = b"_doRTgHZBKcGVjlvpC,@aFSx#DPuNJme&i*MzLOEn)sUrthbf%Y^w.(kIQyXqWA!";
const SYSTEM_PREFIX: &str = "You are a Qoder agent. Use the instructions below and the tools available to you to assist the user.";
const CHAT_PATH: &str = "/algo/api/v2/service/pro/sse/agent_chat_generation";
const MODELS_PATH: &str = "/algo/api/v2/model/list";
const REFRESH_AHEAD_MS: u64 = 5 * 60 * 1000;
const DAY_MS: u64 = 24 * 60 * 60 * 1000;
const DEFAULT_MAX_TOKENS: u64 = 32_000;
const INTERRUPTED: &str = "Tool use was interrupted and did not produce a result.";
const EFFORT_RANK: &[&str] = &[
    "none", "minimal", "low", "medium", "high", "xhigh", "max", "ultra",
];
/// 模型列表缓存多久。
const LISTING_TTL: Duration = Duration::from_secs(5 * 60);

/// 一个站点。
pub struct Qoder {
    id: &'static str,
    name: &'static str,
    web: &'static str,
    openapi: &'static str,
    api: &'static str,
    client_id: &'static str,
    /// 国内版换 job token 失败时直接用 device token 聊天。
    device_fallback: bool,
    default_models: &'static [&'static str],
    note: &'static str,
}

pub static QODER: Qoder = Qoder {
    id: "qoder",
    name: "Qoder",
    web: "https://qoder.com",
    openapi: "https://openapi.qoder.sh",
    api: "https://api3.qoder.sh",
    client_id: "732aef47-9cf2-46a2-95fe-4cebb5d0d1fa",
    device_fallback: false,
    default_models: &[
        "ultimate",
        "performance",
        "efficient",
        "fast",
        "qmodel_38max",
        "kmodel_latest",
        "gmodel",
        "dmodel",
        "mmodel",
    ],
    note: "需要 Qoder 账号（qoder.com；付费套餐可用全部模型，额度用完后只剩免费模型）；在浏览器里授权登录，也可粘贴个人访问令牌。",
};

pub static QODER_CN: Qoder = Qoder {
    id: "qoder-cn",
    name: "Qoder CN",
    web: "https://qoder.cn",
    openapi: "https://openapi.qoder.com.cn",
    api: "https://gateway.qoder.com.cn",
    client_id: "e883ade2-e6e3-4d6d-adf7-f92ceff5fdcb",
    device_fallback: true,
    default_models: &["ultimate", "performance", "efficient", "lite"],
    note: "需要 qoder.cn 账号（国内版，不能用 qoder.com 的账号）；在浏览器里授权登录，也可粘贴个人访问令牌。",
};

// ---------------------------------------------------------------------------
// 加密：MD5、AES-128-CBC、RSA PKCS#1 v1.5（依赖里没有现成的，按需手写）
// ---------------------------------------------------------------------------

fn md5_hex(data: &[u8]) -> String {
    const S: [u32; 64] = [
        7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 5, 9, 14, 20, 5, 9, 14, 20, 5,
        9, 14, 20, 5, 9, 14, 20, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 6, 10,
        15, 21, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21,
    ];
    let k: Vec<u32> = (0..64)
        .map(|i| ((i as f64 + 1.0).sin().abs() * 4_294_967_296.0) as u32)
        .collect();
    let mut message = data.to_vec();
    let bit_len = (data.len() as u64).wrapping_mul(8);
    message.push(0x80);
    while message.len() % 64 != 56 {
        message.push(0);
    }
    message.extend_from_slice(&bit_len.to_le_bytes());
    let (mut a0, mut b0, mut c0, mut d0) = (
        0x6745_2301_u32,
        0xefcd_ab89_u32,
        0x98ba_dcfe_u32,
        0x1032_5476_u32,
    );
    for chunk in message.chunks(64) {
        let m: Vec<u32> = chunk
            .chunks(4)
            .map(|word| u32::from_le_bytes([word[0], word[1], word[2], word[3]]))
            .collect();
        let (mut a, mut b, mut c, mut d) = (a0, b0, c0, d0);
        for i in 0..64 {
            let (f, g) = match i / 16 {
                0 => ((b & c) | (!b & d), i),
                1 => ((d & b) | (!d & c), (5 * i + 1) % 16),
                2 => (b ^ c ^ d, (3 * i + 5) % 16),
                _ => (c ^ (b | !d), (7 * i) % 16),
            };
            let f = f.wrapping_add(a).wrapping_add(k[i]).wrapping_add(m[g]);
            a = d;
            d = c;
            c = b;
            b = b.wrapping_add(f.rotate_left(S[i]));
        }
        a0 = a0.wrapping_add(a);
        b0 = b0.wrapping_add(b);
        c0 = c0.wrapping_add(c);
        d0 = d0.wrapping_add(d);
    }
    [a0, b0, c0, d0]
        .iter()
        .flat_map(|word| word.to_le_bytes())
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// AES-128-CBC + PKCS#7 填充。
fn aes128_cbc(key: &[u8; 16], iv: &[u8; 16], data: &[u8]) -> Vec<u8> {
    use aes_gcm::aes::Aes128;
    use aes_gcm::aes::cipher::{BlockEncrypt, KeyInit, generic_array::GenericArray};
    let cipher = Aes128::new(GenericArray::from_slice(key));
    let pad = 16 - data.len() % 16;
    let mut buffer = data.to_vec();
    buffer.extend(std::iter::repeat_n(pad as u8, pad));
    let mut previous = *iv;
    for chunk in buffer.chunks_mut(16) {
        for (byte, prev) in chunk.iter_mut().zip(previous.iter()) {
            *byte ^= prev;
        }
        cipher.encrypt_block(GenericArray::from_mut_slice(chunk));
        previous.copy_from_slice(chunk);
    }
    buffer
}

/// 只够 RSA 公钥加密用的大整数（小端 u32）。
#[derive(Debug, Clone, PartialEq, Eq)]
struct Big(Vec<u32>);

impl Big {
    fn from_be(bytes: &[u8]) -> Self {
        let mut limbs: Vec<u32> = bytes
            .rchunks(4)
            .map(|chunk| {
                let mut word = [0_u8; 4];
                word[4 - chunk.len()..].copy_from_slice(chunk);
                u32::from_be_bytes(word)
            })
            .collect();
        while limbs.last() == Some(&0) {
            limbs.pop();
        }
        Self(limbs)
    }

    fn to_be(&self, len: usize) -> Vec<u8> {
        let mut out: Vec<u8> = self
            .0
            .iter()
            .rev()
            .flat_map(|limb| limb.to_be_bytes())
            .collect();
        while out.len() > len && out.first() == Some(&0) {
            out.remove(0);
        }
        let mut padded = vec![0_u8; len.saturating_sub(out.len())];
        padded.extend(out);
        padded
    }

    fn bits(&self) -> usize {
        match self.0.last() {
            None => 0,
            Some(top) => (self.0.len() - 1) * 32 + (32 - top.leading_zeros() as usize),
        }
    }

    fn bit(&self, index: usize) -> bool {
        self.0
            .get(index / 32)
            .is_some_and(|limb| (limb >> (index % 32)) & 1 == 1)
    }

    fn ge(&self, other: &Big) -> bool {
        if self.0.len() != other.0.len() {
            return self.0.len() > other.0.len();
        }
        for (left, right) in self.0.iter().rev().zip(other.0.iter().rev()) {
            if left != right {
                return left > right;
            }
        }
        true
    }

    fn sub_assign(&mut self, other: &Big) {
        let mut borrow = 0_i64;
        for index in 0..self.0.len() {
            let right = *other.0.get(index).unwrap_or(&0) as i64;
            let mut value = self.0[index] as i64 - right - borrow;
            borrow = 0;
            if value < 0 {
                value += 1 << 32;
                borrow = 1;
            }
            self.0[index] = value as u32;
        }
        while self.0.last() == Some(&0) {
            self.0.pop();
        }
    }

    fn shl1(&mut self, low_bit: bool) {
        let mut carry = u32::from(low_bit);
        for limb in &mut self.0 {
            let next = *limb >> 31;
            *limb = (*limb << 1) | carry;
            carry = next;
        }
        if carry != 0 {
            self.0.push(carry);
        }
    }

    fn mul(&self, other: &Big) -> Big {
        let mut out = vec![0_u64; self.0.len() + other.0.len() + 1];
        for (i, left) in self.0.iter().enumerate() {
            let mut carry = 0_u64;
            for (j, right) in other.0.iter().enumerate() {
                let value = out[i + j] + (*left as u64) * (*right as u64) + carry;
                out[i + j] = value & 0xffff_ffff;
                carry = value >> 32;
            }
            let mut position = i + other.0.len();
            while carry != 0 {
                let value = out[position] + carry;
                out[position] = value & 0xffff_ffff;
                carry = value >> 32;
                position += 1;
            }
        }
        let mut limbs: Vec<u32> = out.into_iter().map(|limb| limb as u32).collect();
        while limbs.last() == Some(&0) {
            limbs.pop();
        }
        Big(limbs)
    }

    /// 逐位移入取模（慢但足够：一次 1024 位的公钥运算只做十几次）。
    fn rem(&self, modulus: &Big) -> Big {
        let mut out = Big(Vec::new());
        for index in (0..self.bits()).rev() {
            out.shl1(self.bit(index));
            if out.ge(modulus) {
                out.sub_assign(modulus);
            }
        }
        out
    }

    fn mod_pow(&self, exponent: &Big, modulus: &Big) -> Big {
        let mut result = Big(vec![1]).rem(modulus);
        let base = self.rem(modulus);
        for index in (0..exponent.bits()).rev() {
            result = result.mul(&result).rem(modulus);
            if exponent.bit(index) {
                result = result.mul(&base).rem(modulus);
            }
        }
        result
    }
}

/// 读一个 DER 元素：(标签, 内容)。
fn der<'a>(data: &'a [u8], position: &mut usize) -> anyhow::Result<(u8, &'a [u8])> {
    let tag = *data.get(*position).context("DER 数据被截断")?;
    let first = *data.get(*position + 1).context("DER 数据被截断")?;
    *position += 2;
    let len = if first & 0x80 == 0 {
        first as usize
    } else {
        let count = (first & 0x7f) as usize;
        let mut len = 0_usize;
        for _ in 0..count {
            len = (len << 8) | *data.get(*position).context("DER 数据被截断")? as usize;
            *position += 1;
        }
        len
    };
    let content = data
        .get(*position..*position + len)
        .context("DER 数据被截断")?;
    *position += len;
    Ok((tag, content))
}

/// SubjectPublicKeyInfo → (n, e)。
fn rsa_public_key(spki: &[u8]) -> anyhow::Result<(Big, Big)> {
    let mut position = 0;
    let (_, outer) = der(spki, &mut position)?;
    let mut inner = 0;
    let _algorithm = der(outer, &mut inner)?;
    let (_, bits) = der(outer, &mut inner)?;
    let key = bits.get(1..).context("公钥格式不对")?;
    let mut position = 0;
    let (_, sequence) = der(key, &mut position)?;
    let mut inner = 0;
    let (_, n) = der(sequence, &mut inner)?;
    let (_, e) = der(sequence, &mut inner)?;
    Ok((Big::from_be(n), Big::from_be(e)))
}

/// RSA PKCS#1 v1.5 公钥加密。
fn rsa_encrypt(message: &[u8]) -> anyhow::Result<Vec<u8>> {
    let spki = base64::engine::general_purpose::STANDARD.decode(RSA_KEY)?;
    let (n, e) = rsa_public_key(&spki)?;
    let size = n.bits().div_ceil(8);
    anyhow::ensure!(message.len() + 11 <= size, "要加密的内容太长");
    let mut block = vec![0_u8, 2];
    let mut padding = Vec::new();
    while padding.len() < size - message.len() - 3 {
        padding.extend(
            b_shared::random_bytes(32)
                .into_iter()
                .filter(|byte| *byte != 0),
        );
    }
    padding.truncate(size - message.len() - 3);
    block.extend(padding);
    block.push(0);
    block.extend_from_slice(message);
    Ok(Big::from_be(&block).mod_pow(&e, &n).to_be(size))
}

// ---------------------------------------------------------------------------
// COSY 签名与请求体编码
// ---------------------------------------------------------------------------

/// 打乱字母表的 base64（`$` 补位），再交换首尾三分之一。
fn encode_body(text: &str) -> String {
    let standard = base64::engine::general_purpose::STANDARD.encode(text.as_bytes());
    let mapped: String = standard
        .bytes()
        .map(|byte| match byte {
            b'=' => '$',
            _ => {
                let index = match byte {
                    b'A'..=b'Z' => byte - b'A',
                    b'a'..=b'z' => byte - b'a' + 26,
                    b'0'..=b'9' => byte - b'0' + 52,
                    b'+' => 62,
                    _ => 63,
                };
                ALPHABET[index as usize] as char
            }
        })
        .collect();
    swap_thirds(&mapped)
}

fn swap_thirds(text: &str) -> String {
    let third = text.len() / 3;
    if third == 0 {
        return text.to_string();
    }
    format!(
        "{}{}{}",
        &text[text.len() - third..],
        &text[third..text.len() - third],
        &text[..third]
    )
}

#[cfg(test)]
fn decode_body(wire: &str) -> String {
    let swapped = swap_thirds(wire);
    let standard: String = swapped
        .bytes()
        .map(|byte| {
            if byte == b'$' {
                return '=';
            }
            let index = ALPHABET.iter().position(|item| *item == byte).unwrap_or(0) as u8;
            match index {
                0..=25 => (b'A' + index) as char,
                26..=51 => (b'a' + index - 26) as char,
                52..=61 => (b'0' + index - 52) as char,
                62 => '+',
                _ => '/',
            }
        })
        .collect();
    String::from_utf8(
        base64::engine::general_purpose::STANDARD
            .decode(standard)
            .unwrap_or_default(),
    )
    .unwrap_or_default()
}

fn machine_os() -> String {
    let arch = match std::env::consts::ARCH {
        "x86_64" => "x86_64",
        "aarch64" => "aarch64",
        other => other,
    };
    let platform = match std::env::consts::OS {
        "macos" => "darwin",
        "windows" => "win32",
        other => other,
    };
    format!("{arch}_{platform}")
}

/// 签名要用的账号信息。
struct CosyUser<'a> {
    uid: &'a str,
    name: &'a str,
    email: &'a str,
    token: &'a str,
    machine_id: &'a str,
}

impl<'a> CosyUser<'a> {
    fn from_credentials(credentials: &'a Value) -> anyhow::Result<Self> {
        let get = |key: &str| {
            credentials
                .get(key)
                .and_then(Value::as_str)
                .unwrap_or_default()
        };
        let user = Self {
            uid: get("uid"),
            name: get("name"),
            email: get("email"),
            token: get("access"),
            machine_id: get("machine_id"),
        };
        if user.token.is_empty() || user.machine_id.is_empty() {
            return Err(fail(401, "Qoder 凭据不完整，请重新登录"));
        }
        Ok(user)
    }
}

/// COSY 签名头。`body` 是线上实际发送的内容（GET 为空串），`path` 是去掉 `/algo` 的路径。
fn cosy_headers(
    user: &CosyUser<'_>,
    body: &str,
    path: &str,
    ts: u64,
) -> anyhow::Result<Vec<(&'static str, String)>> {
    let raw = json!({
        "uid": user.uid,
        "aid": "",
        "name": user.name,
        "email": user.email,
        "security_oauth_token": user.token,
    })
    .to_string();
    let key_text = &uuid::Uuid::new_v4().simple().to_string()[..16];
    let mut key = [0_u8; 16];
    key.copy_from_slice(key_text.as_bytes());
    let standard = base64::engine::general_purpose::STANDARD;
    let info = standard.encode(aes128_cbc(&key, &key, raw.as_bytes()));
    let wrapped = standard.encode(rsa_encrypt(&key)?);
    let payload = standard.encode(
        json!({
            "version": "v1",
            "requestId": uuid::Uuid::new_v4().to_string(),
            "info": info,
            "cosyVersion": COSY_VERSION,
            "ideVersion": "",
        })
        .to_string(),
    );
    let path = path.strip_prefix("/algo").unwrap_or(path);
    let signature = md5_hex(format!("{payload}\n{wrapped}\n{ts}\n{body}\n{path}").as_bytes());
    let machine = user.machine_id.to_string();
    Ok(vec![
        ("accept", "application/json".into()),
        ("accept-encoding", "identity".into()),
        ("content-type", "application/json".into()),
        (
            "authorization",
            format!("Bearer COSY.{payload}.{signature}"),
        ),
        ("cosy-business-product", "app".into()),
        ("cosy-business-type", "agent".into()),
        ("cosy-clientip", machine.clone()),
        ("cosy-clienttype", "10".into()),
        ("cosy-data-policy", "disagree".into()),
        ("cosy-date", ts.to_string()),
        ("cosy-key", wrapped),
        ("cosy-machineid", machine.clone()),
        ("cosy-machinetoken", machine),
        ("cosy-machinetype", "5".into()),
        ("cosy-machineos", machine_os()),
        ("cosy-scene", "app".into()),
        ("cosy-user", user.uid.to_string()),
        ("cosy-version", COSY_VERSION.into()),
        ("login-version", "v2".into()),
    ])
}

fn now_secs() -> u64 {
    oauth::now_ms() / 1000
}

// ---------------------------------------------------------------------------
// 账号接口
// ---------------------------------------------------------------------------

/// openapi 的 JSON 请求：返回 (状态码, JSON)。
async fn openapi(
    http: &reqwest::Client,
    url: &str,
    token: Option<&str>,
    body: Option<&Value>,
) -> anyhow::Result<(u16, Value)> {
    let mut request = match body {
        Some(body) => http.post(url).json(body),
        None => http.get(url),
    }
    .header("accept", "application/json")
    .timeout(Duration::from_secs(20));
    if let Some(token) = token {
        request = request.header("authorization", format!("Bearer {token}"));
    }
    let response = request
        .send()
        .await
        .with_context(|| format!("无法连接 {url}"))?;
    let status = response.status().as_u16();
    let text = response.text().await.unwrap_or_default();
    Ok((status, serde_json::from_str(&text).unwrap_or(Value::Null)))
}

/// job token 的过期时间：`expire_time` / `expires_at`（秒、毫秒或日期），否则 `expires_in`（毫秒），默认一天。
fn job_expiry(value: &Value) -> u64 {
    let now = oauth::now_ms();
    for key in ["expire_time", "expires_at"] {
        let at = match value.get(key) {
            Some(Value::Number(number)) => number.as_u64().map(b_shared::to_ms),
            Some(Value::String(text)) => text
                .trim()
                .parse::<u64>()
                .ok()
                .map(b_shared::to_ms)
                .or_else(|| b_shared::parse_iso_ms(text)),
            _ => None,
        };
        if let Some(at) = at.filter(|at| *at > now) {
            return at;
        }
    }
    b_shared::u64_at(value, &["expires_in"])
        .map(|ms| now + ms)
        .unwrap_or(now + DAY_MS)
}

/// device token 的过期时间：`expires_at`（日期），否则 `expires_in`（秒），默认一天。
fn device_expiry(value: &Value) -> u64 {
    let now = oauth::now_ms();
    if let Some(at) = value
        .get("expires_at")
        .and_then(Value::as_str)
        .and_then(b_shared::parse_iso_ms)
        .filter(|at| *at > now)
    {
        return at;
    }
    b_shared::u64_at(value, &["expires_in"])
        .map(|seconds| now + seconds * 1000)
        .unwrap_or(now + DAY_MS)
}

/// 手机号打码：13800138000 → 138****8000。
fn mask_phone(phone: &str) -> String {
    let digits: Vec<char> = phone.chars().collect();
    if digits.len() < 7 {
        return phone.to_string();
    }
    format!(
        "{}****{}",
        digits[..3].iter().collect::<String>(),
        digits[digits.len() - 4..].iter().collect::<String>()
    )
}

impl Qoder {
    /// userinfo → (uid, email, name, 账号名)。
    async fn userinfo(
        &self,
        http: &reqwest::Client,
        token: &str,
    ) -> anyhow::Result<(String, String, String, String)> {
        let (status, info) = openapi(
            http,
            &format!("{}/api/v1/userinfo", self.openapi),
            Some(token),
            None,
        )
        .await?;
        if !(200..300).contains(&status) {
            return Err(fail(
                status,
                format!("{} 读取账号信息失败（{status}）", self.name),
            ));
        }
        let uid = b_shared::str_at(&info, &["id", "user_id", "uid"])
            .unwrap_or_default()
            .to_string();
        let email = b_shared::str_at(&info, &["email"])
            .unwrap_or_default()
            .to_string();
        let name = b_shared::str_at(&info, &["name"])
            .unwrap_or_default()
            .to_string();
        let phone = b_shared::str_at(
            &info,
            &["security_mobile", "phoneNumber", "phone", "mobile"],
        )
        .map(mask_phone);
        let account = if !email.is_empty() {
            email.clone()
        } else {
            phone.unwrap_or_else(|| {
                if name.is_empty() {
                    uid.clone()
                } else {
                    name.clone()
                }
            })
        };
        Ok((uid, email, name, account))
    }

    fn api_url(&self, path: &str) -> String {
        format!("{}{path}", self.api)
    }

    /// 拉模型列表（带缓存）：每一项是列表里的原文（聊天要原样带上）。
    async fn listing(
        &self,
        http: &reqwest::Client,
        credentials: &Value,
        fresh: bool,
    ) -> anyhow::Result<Vec<Value>> {
        type Cache = Mutex<HashMap<String, (Instant, Vec<Value>)>>;
        static CACHE: OnceLock<Cache> = OnceLock::new();
        let cache = CACHE.get_or_init(Default::default);
        let user = CosyUser::from_credentials(credentials)?;
        let cache_key = format!("{}:{}", self.id, user.uid);
        if !fresh
            && let Some((at, entries)) = cache
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .get(&cache_key)
            && at.elapsed() < LISTING_TTL
        {
            return Ok(entries.clone());
        }
        let headers = cosy_headers(&user, "", MODELS_PATH, now_secs())?;
        let mut request = http
            .get(format!("{}?Encode=1", self.api_url(MODELS_PATH)))
            .timeout(Duration::from_secs(20));
        for (key, value) in &headers {
            request = request.header(*key, value);
        }
        let response = request
            .send()
            .await
            .map_err(|error| fail(502, format!("无法连接 {}：{error}", self.name)))?;
        let status = response.status().as_u16();
        let text = response.text().await.unwrap_or_default();
        if !(200..300).contains(&status) {
            return Err(failure(self.name, status, &text));
        }
        let value: Value = serde_json::from_str(&text)
            .map_err(|_| fail(502, format!("{} 的模型列表不是 JSON", self.name)))?;
        let entries = scene_models(&value);
        cache
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(cache_key, (Instant::now(), entries.clone()));
        Ok(entries)
    }
}

/// 模型列表：以 `app` 场景为主，再补上只在 `chat` 场景里的；去掉没启用的和 auto / default。
fn scene_models(listing: &Value) -> Vec<Value> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for scene in ["app", "chat"] {
        for entry in listing
            .get(scene)
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let key = entry.get("key").and_then(Value::as_str).unwrap_or_default();
            if matches!(key, "" | "auto" | "default")
                || entry.get("enable").and_then(Value::as_bool) == Some(false)
                || !seen.insert(key.to_string())
            {
                continue;
            }
            out.push(entry.clone());
        }
    }
    out
}

// ---------------------------------------------------------------------------
// 请求体
// ---------------------------------------------------------------------------

/// 按模型的思考配置选：是否思考、哪一档（空串为不指定）。
fn effort_for(asked: Option<&str>, entry: &Value) -> (bool, String) {
    let mut want = asked.unwrap_or_default().to_ascii_lowercase();
    if !EFFORT_RANK.contains(&want.as_str()) {
        want.clear();
    }
    let rank = |level: &str| {
        EFFORT_RANK
            .iter()
            .position(|item| *item == level)
            .unwrap_or(99)
    };
    let (thinks, always, mut efforts, default) = match entry.get("thinking_config") {
        Some(config) if config.is_object() => {
            let enabled = config.get("enabled").filter(|value| !value.is_null());
            let thinks = enabled.is_some();
            let always = thinks && config.get("disabled").is_none_or(Value::is_null);
            let mut efforts = Vec::new();
            let mut default = String::new();
            if let Some(map) = enabled
                .and_then(|enabled| enabled.get("efforts"))
                .and_then(Value::as_object)
            {
                for (name, detail) in map {
                    efforts.push(name.clone());
                    if detail.get("is_default").and_then(Value::as_bool) == Some(true) {
                        default = name.clone();
                    }
                }
            }
            (thinks, always, efforts, default)
        }
        _ => (
            entry.get("is_reasoning").and_then(Value::as_bool) == Some(true),
            false,
            entry
                .get("reasoning_efforts")
                .and_then(Value::as_array)
                .map(|list| {
                    list.iter()
                        .filter_map(Value::as_str)
                        .map(str::to_string)
                        .collect()
                })
                .unwrap_or_default(),
            String::new(),
        ),
    };
    efforts.sort_by_key(|level| rank(level));
    if !thinks || (want == "none" && !always) {
        return (false, String::new());
    }
    if efforts.is_empty() {
        return (true, String::new());
    }
    if want.is_empty() {
        return (true, default);
    }
    if want == "none" {
        return (true, efforts[0].clone());
    }
    if want == "ultra" && !efforts.contains(&want) {
        want = "max".into();
    }
    if efforts.contains(&want) {
        return (true, want);
    }
    let at = rank(&want) as i64;
    let best = efforts
        .iter()
        .filter(|level| level.as_str() != "none")
        .min_by_key(|level| {
            let index = rank(level) as i64;
            // 距离相同时取较高的一档。
            ((index - at).abs(), -index)
        })
        .cloned()
        .unwrap_or(want);
    (true, best)
}

/// 上下文窗口：默认窗口放得下就用默认，否则用放得下的最小窗口，都放不下用最大的。
fn window_for(entry: &Value, need: u64) -> Option<u64> {
    let mut windows = Vec::new();
    let mut default = 0;
    if let Some(config) = entry.get("context_config").and_then(Value::as_object) {
        for detail in config.values() {
            let Some(count) = b_shared::u64_at(detail, &["token_count"]).filter(|count| *count > 0)
            else {
                continue;
            };
            if !windows.contains(&count) {
                windows.push(count);
            }
            if matches!(detail.get("is_default"), Some(Value::Bool(true)))
                || b_shared::u64_at(detail, &["is_default"]) == Some(1)
            {
                default = count;
            }
        }
    }
    if windows.is_empty() {
        for count in entry
            .get("available_context_windows")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_u64)
        {
            if count > 0 && !windows.contains(&count) {
                windows.push(count);
            }
        }
        default = b_shared::u64_at(entry, &["default_context_window"]).unwrap_or(0);
        if !windows.contains(&default) {
            default = 0;
        }
    }
    windows.sort_unstable();
    let max_input = b_shared::u64_at(entry, &["max_input_tokens"]).unwrap_or(0);
    if windows.is_empty() {
        return (max_input > 0).then_some(max_input);
    }
    let base = if default > 0 { default } else { max_input };
    if base > 0 && need <= base {
        return Some(if default > 0 {
            default
        } else {
            *windows.last().unwrap_or(&base)
        });
    }
    windows
        .iter()
        .copied()
        .find(|window| *window >= need)
        .or_else(|| windows.last().copied())
}

fn text_blocks(content: Option<&Value>) -> Vec<Value> {
    match content {
        Some(Value::String(text)) if !text.is_empty() => {
            vec![json!({ "type": "text", "text": text })]
        }
        Some(Value::Array(parts)) => parts
            .iter()
            .filter_map(|part| match part.get("type").and_then(Value::as_str) {
                Some("image_url") => {
                    let url = part
                        .pointer("/image_url/url")
                        .or_else(|| part.get("image_url"))
                        .and_then(Value::as_str)?;
                    Some(json!({ "type": "image_url", "image_url": { "url": url } }))
                }
                _ => {
                    let text = part.get("text").and_then(Value::as_str)?;
                    Some(json!({ "type": "text", "text": text }))
                }
            })
            .collect(),
        _ => Vec::new(),
    }
}

fn reasoning_of(message: &Value) -> Option<String> {
    b_shared::str_at(message, &["reasoning_content", "reasoning"])
        .map(str::to_string)
        .or_else(|| {
            let text: String = message
                .get("reasoning_details")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|detail| detail.get("text").and_then(Value::as_str))
                .collect();
            (!text.trim().is_empty()).then_some(text)
        })
}

/// Chat 消息 → Qoder 消息。工具结果必须对上前一条 assistant 的调用，否则整个请求被拒：
/// 对不上的按顺序分给还没回答的调用，没有可分的并进下一条 user；没回答的调用补「被打断」。
fn qoder_messages(messages: &[Value]) -> Vec<Value> {
    let mut out: Vec<Value> = Vec::new();
    let mut pending: Vec<String> = Vec::new();
    let mut orphan_texts: Vec<String> = Vec::new();
    let flush = |out: &mut Vec<Value>, pending: &mut Vec<String>| {
        for id in pending.drain(..) {
            out.push(json!({ "role": "tool", "tool_call_id": id, "content": INTERRUPTED }));
        }
    };
    for message in messages {
        let content = message.get("content");
        match message
            .get("role")
            .and_then(Value::as_str)
            .unwrap_or("user")
        {
            "system" | "developer" => {}
            "assistant" => {
                flush(&mut out, &mut pending);
                let calls: Vec<Value> = message
                    .get("tool_calls")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .enumerate()
                    .map(|(index, call)| {
                        let name = call.pointer("/function/name").and_then(Value::as_str).unwrap_or_default();
                        let arguments = match call.pointer("/function/arguments") {
                            Some(Value::String(text)) => text.clone(),
                            Some(other) => other.to_string(),
                            None => "{}".to_string(),
                        };
                        let id = call
                            .get("id")
                            .and_then(Value::as_str)
                            .filter(|id| !id.is_empty())
                            .map(str::to_string)
                            .unwrap_or_else(|| {
                                let hash = sha256_hex(format!("{index}\0{name}\0{arguments}").as_bytes());
                                format!("call_{}", &hash[..32])
                            });
                        json!({ "id": id, "type": "function", "function": { "name": name, "arguments": arguments } })
                    })
                    .collect();
                let mut converted = if calls.is_empty() {
                    let blocks: Vec<Value> = text_blocks(content)
                        .into_iter()
                        .filter(|block| block.get("type").and_then(Value::as_str) == Some("text"))
                        .collect();
                    json!({ "role": "assistant", "content": blocks })
                } else {
                    pending = calls
                        .iter()
                        .filter_map(|call| {
                            call.get("id").and_then(Value::as_str).map(str::to_string)
                        })
                        .collect();
                    json!({ "role": "assistant", "content": content_text(content), "tool_calls": calls })
                };
                if let Some(reasoning) = reasoning_of(message) {
                    converted["reasoning_content"] = json!(reasoning);
                }
                out.push(converted);
            }
            "tool" => {
                let wanted = message
                    .get("tool_call_id")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let text = content_text(content);
                let position = pending
                    .iter()
                    .position(|id| id == wanted)
                    .or_else(|| (!pending.is_empty()).then_some(0));
                match position {
                    Some(position) => {
                        let id = pending.remove(position);
                        out.push(json!({ "role": "tool", "tool_call_id": id, "content": text }));
                    }
                    None => orphan_texts.push(text),
                }
            }
            _ => {
                flush(&mut out, &mut pending);
                let mut blocks = text_blocks(content);
                if !orphan_texts.is_empty() {
                    let joined = std::mem::take(&mut orphan_texts).join("\n\n");
                    blocks.insert(
                        0,
                        json!({ "type": "text", "text": format!("Tool output:\n{joined}") }),
                    );
                }
                if blocks.is_empty() {
                    continue;
                }
                out.push(json!({ "role": "user", "content": blocks, "contents": blocks }));
            }
        }
    }
    if !orphan_texts.is_empty() {
        let blocks = vec![
            json!({ "type": "text", "text": format!("Tool output:\n{}", orphan_texts.join("\n\n")) }),
        ];
        out.push(json!({ "role": "user", "content": blocks, "contents": blocks }));
    }
    // 最后一条是 user 时给它的最后一块打缓存标记。
    if let Some(last) = out.last_mut()
        && last.get("role").and_then(Value::as_str) == Some("user")
    {
        for key in ["content", "contents"] {
            if let Some(block) = last
                .get_mut(key)
                .and_then(Value::as_array_mut)
                .and_then(|blocks| blocks.last_mut())
            {
                block["cache_control"] = json!({ "type": "ephemeral" });
            }
        }
    }
    out
}

/// 国内版的部分模型要求 oneOf 根部写明 object。
fn typed_unions(schema: &Value) -> Value {
    let mut schema = schema.clone();
    let all_objects = schema
        .get("oneOf")
        .and_then(Value::as_array)
        .is_some_and(|branches| {
            !branches.is_empty()
                && branches.iter().all(|branch| {
                    branch.get("type").and_then(Value::as_str) == Some("object")
                        || branch.get("properties").is_some()
                })
        });
    if all_objects && schema.get("type").is_none() {
        schema["type"] = json!("object");
    }
    schema
}

/// 会话 id：同一会话的每轮都一样。
fn session_of(request: &Value) -> String {
    if let Some(seed) = b_shared::str_at(request, &["prompt_cache_key", "user"]) {
        return uuid_from_seed(seed);
    }
    let first_user = request
        .get("messages")
        .and_then(Value::as_array)
        .and_then(|messages| {
            messages
                .iter()
                .find(|message| message.get("role").and_then(Value::as_str) == Some("user"))
        })
        .and_then(|message| message.get("content"))
        .cloned()
        .unwrap_or(Value::Null);
    uuid_from_seed(&format!(
        "chat\0{}",
        json!([system_texts(request), first_user])
    ))
}

/// Chat 请求 → Qoder 请求体（未编码）。
fn qoder_body(request: &Value, entry: &Value) -> Value {
    let messages = request
        .get("messages")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let tools_on = request.get("tool_choice").and_then(Value::as_str) != Some("none");
    let tools: Vec<Value> = if tools_on {
        request
            .get("tools")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|tool| {
                let function = tool.get("function")?;
                let name = function.get("name").and_then(Value::as_str).filter(|name| !name.is_empty())?;
                Some(json!({ "type": "function", "function": {
                    "name": name,
                    "description": function.get("description").and_then(Value::as_str).unwrap_or_default(),
                    "parameters": function.get("parameters").map(typed_unions).unwrap_or(json!({ "type": "object", "properties": {} })),
                }}))
            })
            .collect()
    } else {
        Vec::new()
    };
    let system = system_texts(request).join("\n\n");
    let mut system_text = if system.is_empty() {
        SYSTEM_PREFIX.to_string()
    } else {
        format!("{SYSTEM_PREFIX}\n\n{system}")
    };
    if request.get("tool_choice").and_then(Value::as_str) == Some("required") && !tools.is_empty() {
        system_text.push_str("\nYou must call an available function in this response.");
    }
    let sys = json!({ "type": "text", "text": system_text });
    let converted = qoder_messages(&messages);
    let max_tokens = b_shared::max_tokens(request).unwrap_or(DEFAULT_MAX_TOKENS);
    let (thinking, effort) = effort_for(b_shared::reasoning_effort(request).as_deref(), entry);
    let mut parameters = json!({ "enable_thinking": thinking, "max_tokens": max_tokens });
    if !effort.is_empty() {
        parameters["reasoning_effort"] = json!(effort);
    }
    let need = (json!({ "messages": converted, "tools": tools })
        .to_string()
        .len() as u64)
        .div_ceil(3)
        + max_tokens;
    if let Some(window) = window_for(entry, need) {
        parameters["context_length"] = json!(window);
    }
    let session = session_of(request);
    // 同一个用户提问的整轮工具循环共用一个任务 id。
    let last_user = messages
        .iter()
        .rposition(|message| message.get("role").and_then(Value::as_str) == Some("user"));
    let last_user_text = last_user
        .map(|index| content_text(messages[index].get("content")))
        .unwrap_or_default();
    let task = uuid_from_seed(&format!(
        "{session}\0{}\0{last_user_text}",
        last_user.unwrap_or(0)
    ));
    let answered = messages
        .last()
        .and_then(|message| message.get("role"))
        .and_then(Value::as_str)
        != Some("user");
    let request_id = uuid::Uuid::new_v4().to_string();
    let mut body = json!({
        "request_id": request_id,
        "request_set_id": task,
        "chat_record_id": request_id,
        "session_id": session,
        "parameters": parameters,
        "business": {
            "product": "app",
            "version": COSY_VERSION,
            "type": "agent",
            "id": task,
            "name": last_user_text.chars().take(10).collect::<String>(),
            "begin_at": oauth::now_ms(),
            "stage": if answered { "processing" } else { "start" },
        },
        "agent_id": "agent_common",
        "task_id": "common",
        "session_type": "app",
        "model_config": entry,
        "system": [sys],
        "messages": std::iter::once(json!({ "role": "system", "content": [sys] }))
            .chain(converted)
            .collect::<Vec<_>>(),
    });
    if !tools.is_empty() {
        body["tools"] = json!(tools);
    }
    body
}

// ---------------------------------------------------------------------------
// 错误
// ---------------------------------------------------------------------------

const QUOTA_CODES: &[&str] = &[
    "110", "112", "113", "114", "115", "116", "117", "118", "119", "122",
];
const REQUEST_CODES: &[&str] = &[
    "406", "416", "430", "80411", "80412", "100400", "100401", "100403",
];

fn code_text(value: &Value) -> String {
    match value.get("code") {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Number(number)) => number.to_string(),
        _ => String::new(),
    }
}

/// Qoder 的错误体 → 带状态码的失败。
fn failure(vendor: &str, status: u16, body: &str) -> anyhow::Error {
    let mut value: Value = serde_json::from_str(body).unwrap_or(Value::Null);
    // 外层只是包装：message 里的 JSON 才是真正的错误（排队 10605 常这样来）。
    if let Some(inner) = value
        .get("message")
        .and_then(Value::as_str)
        .and_then(|text| serde_json::from_str::<Value>(text).ok())
        .filter(|inner| inner.get("code").is_some())
    {
        let outer = code_text(&value);
        if outer.is_empty() || outer == "401" || outer == "403" {
            value = inner;
        }
    }
    let code = code_text(&value);
    let mut message = b_shared::str_at(&value, &["message", "msg", "/error/message"])
        .map(str::to_string)
        .unwrap_or_else(|| body.chars().take(300).collect());
    if let Some(detail) = value
        .get("details")
        .and_then(Value::as_str)
        .and_then(|text| serde_json::from_str::<Value>(text).ok())
        .and_then(|details| b_shared::str_at(&details, &["/error/message"]).map(str::to_string))
    {
        message = format!("{message}：{detail}");
    }
    let queued = code == "10605"
        || value.pointer("/queue/isQueued").and_then(Value::as_bool) == Some(true)
        || value
            .pointer("/queue/serviceAvailable")
            .and_then(Value::as_bool)
            == Some(false);
    let lower = message.to_ascii_lowercase();
    let (status, label) = if queued {
        (503, "排队中，稍后再试")
    } else if QUOTA_CODES.contains(&code.as_str()) {
        (429, "额度已用完")
    } else if REQUEST_CODES.contains(&code.as_str()) {
        (400, "请求被拒绝")
    } else if lower.contains("error in upstream response") {
        (502, "模型服务出错")
    } else if status == 401 || status == 403 || code == "105" {
        (401, "登录已失效，请重新登录")
    } else if status == 429 || lower.contains("quota") {
        (429, "额度已用完或被限流")
    } else if (400..600).contains(&status) {
        (status, "请求失败")
    } else {
        (502, "请求失败")
    };
    fail(status, format!("{vendor} {label}（{status}）：{message}"))
}

// ---------------------------------------------------------------------------
// 答复：正文里的工具调用
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
enum Piece {
    Text(String),
    Call { name: String, arguments: String },
}

/// 起始标签与对应的结束标签。裸标签（除 `<tool_call>` 外）只在声明了工具时识别。
const MARKERS: &[(&str, &str, bool)] = &[
    ("<tool_call>", "</tool_call>", false),
    ("<function_calls>", "</function_calls>", true),
    // 带命名空间前缀的 Anthropic 写法（拆开写，免得源码里出现完整标签）。
    (
        concat!("<", "antml", ":function_calls>"),
        concat!("</", "antml", ":function_calls>"),
        true,
    ),
    ("<invoke name=", "</invoke>", true),
    (
        concat!("<", "antml", ":invoke name="),
        concat!("</", "antml", ":invoke>"),
        true,
    ),
    ("<function=", "</function>", true),
];

/// 从流式正文里切出工具调用：遇到可能的起始标签先扣住，等结束标签齐了再解析；
/// 代码块里的不算；解析不了的 `<tool_call>` 视为上游答复残缺。
#[derive(Debug, Default)]
struct Splitter {
    /// 声明的工具 → 参数 schema。
    tools: HashMap<String, Value>,
    pending: String,
    in_fence: bool,
}

impl Splitter {
    fn new(request: &Value) -> Self {
        let tools = request
            .get("tools")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|tool| {
                let function = tool.get("function")?;
                Some((
                    function.get("name")?.as_str()?.to_string(),
                    function.get("parameters").cloned().unwrap_or(Value::Null),
                ))
            })
            .collect();
        Self {
            tools,
            ..Self::default()
        }
    }

    fn active(&self, bare: bool) -> bool {
        !bare || !self.tools.is_empty()
    }

    fn emit_text(&mut self, text: &str, out: &mut Vec<Piece>) {
        if text.is_empty() {
            return;
        }
        if text.matches("```").count() % 2 == 1 {
            self.in_fence = !self.in_fence;
        }
        match out.last_mut() {
            Some(Piece::Text(previous)) => previous.push_str(text),
            _ => out.push(Piece::Text(text.to_string())),
        }
    }

    /// 第一个生效的起始标签：(位置, 标记序号)。
    fn find_marker(&self) -> Option<(usize, usize)> {
        let mut best: Option<(usize, usize)> = None;
        for (index, (start, _, bare)) in MARKERS.iter().enumerate() {
            if !self.active(*bare) {
                continue;
            }
            let mut from = 0;
            while let Some(found) = self.pending[from..].find(start) {
                let position = from + found;
                let fence =
                    self.in_fence ^ (self.pending[..position].matches("```").count() % 2 == 1);
                if !fence {
                    if best.is_none_or(|(at, _)| position < at) {
                        best = Some((position, index));
                    }
                    break;
                }
                from = position + 1;
            }
        }
        best
    }

    /// 末尾可能是某个起始标签的前半截：返回要扣住的长度。
    fn held_suffix(&self) -> usize {
        let text = self.pending.as_str();
        let Some(open) = text.rfind('<') else {
            return 0;
        };
        let tail = &text[open..];
        let partial = MARKERS.iter().any(|(start, _, bare)| {
            self.active(*bare) && start.starts_with(tail) && tail.len() < start.len()
        });
        if partial { tail.len() } else { 0 }
    }

    fn push(&mut self, text: &str) -> anyhow::Result<Vec<Piece>> {
        self.pending.push_str(text);
        let mut out = Vec::new();
        loop {
            let Some((position, index)) = self.find_marker() else {
                let hold = self.held_suffix();
                let ready = self.pending.len() - hold;
                let text: String = self.pending.drain(..ready).collect();
                self.emit_text(&text, &mut out);
                return Ok(out);
            };
            let before: String = self.pending.drain(..position).collect();
            self.emit_text(&before, &mut out);
            let (_, end, _) = MARKERS[index];
            let Some(close) = self.pending.find(end) else {
                return Ok(out);
            };
            let block: String = self.pending.drain(..close + end.len()).collect();
            match self.parse_block(&block, index)? {
                Some(calls) => out.extend(calls),
                None => self.emit_text(&block, &mut out),
            }
        }
    }

    fn finish(&mut self) -> anyhow::Result<Vec<Piece>> {
        let mut out = Vec::new();
        let rest = std::mem::take(&mut self.pending);
        if rest.is_empty() {
            return Ok(out);
        }
        if let Some(inner) = rest.strip_prefix("<tool_call>") {
            // 缺结束标签但 JSON 已经完整：仍算有效调用。
            if let Some(call) = self.parse_json_call(inner.trim())? {
                out.push(call);
                return Ok(out);
            }
            return Err(fail(502, "Qoder 返回的工具调用不完整"));
        }
        let bare = MARKERS
            .iter()
            .any(|(start, _, bare)| *bare && self.active(true) && rest.starts_with(start));
        if bare && rest.contains("parameter") {
            return Err(fail(502, "Qoder 返回的工具调用被截断"));
        }
        self.emit_text(&rest, &mut out);
        Ok(out)
    }

    /// 解析一段完整的标签块；不是有效调用（裸标签对应的工具没声明）时返回 None。
    fn parse_block(&self, block: &str, index: usize) -> anyhow::Result<Option<Vec<Piece>>> {
        let (start, end, _) = MARKERS[index];
        if index == 0 {
            let inner = block[start.len()..block.len() - end.len()].trim();
            if inner.starts_with('{') {
                return match self.parse_json_call(inner)? {
                    Some(call) => Ok(Some(vec![call])),
                    None => Err(fail(502, "Qoder 返回了无法解析的工具调用")),
                };
            }
            if inner.contains("<function=") {
                return match self.parse_function(inner, false) {
                    Some(call) => Ok(Some(vec![call])),
                    None => Err(fail(502, "Qoder 返回了无法解析的工具调用")),
                };
            }
            return Ok(None);
        }
        if start.contains("function=") {
            return Ok(self.parse_function(block, true).map(|call| vec![call]));
        }
        let calls = self.parse_invokes(block);
        Ok((!calls.is_empty()).then_some(calls))
    }

    fn parse_json_call(&self, inner: &str) -> anyhow::Result<Option<Piece>> {
        let Ok(value) = serde_json::from_str::<Value>(inner) else {
            return Ok(None);
        };
        let Some(name) = value.get("name").and_then(Value::as_str) else {
            return Ok(None);
        };
        let arguments = match value.get("arguments").or_else(|| value.get("parameters")) {
            Some(Value::Object(map)) => Value::Object(map.clone()).to_string(),
            Some(Value::String(text)) => match serde_json::from_str::<Value>(text) {
                Ok(parsed @ Value::Object(_)) => parsed.to_string(),
                _ => return Ok(None),
            },
            None => "{}".to_string(),
            _ => return Ok(None),
        };
        Ok(Some(Piece::Call {
            name: name.to_string(),
            arguments,
        }))
    }

    /// 参数值：schema 说是字符串时取原文，否则先试 JSON。
    fn value_of(&self, tool: &str, key: &str, raw: &str) -> Value {
        let raw = raw.strip_prefix('\n').unwrap_or(raw);
        let raw = raw.strip_suffix('\n').unwrap_or(raw);
        let is_string = self
            .tools
            .get(tool)
            .and_then(|schema| schema.pointer(&format!("/properties/{key}/type")))
            .and_then(Value::as_str)
            == Some("string");
        if is_string {
            return json!(raw);
        }
        serde_json::from_str(raw).unwrap_or_else(|_| json!(raw))
    }

    /// `<function=NAME><parameter=K>V</parameter>…</function>`。
    fn parse_function(&self, block: &str, bare: bool) -> Option<Piece> {
        let start = block.find("<function=")? + "<function=".len();
        let name_end = start + block[start..].find('>')?;
        let name = block[start..name_end].trim().to_string();
        if bare && !self.tools.contains_key(&name) {
            return None;
        }
        let body_end = block[name_end..]
            .find("</function>")
            .map(|at| name_end + at)
            .unwrap_or(block.len());
        let body = &block[name_end + 1..body_end];
        let mut arguments = serde_json::Map::new();
        let mut rest = body;
        while let Some(at) = rest.find("<parameter=") {
            let after = &rest[at + "<parameter=".len()..];
            let key_end = after.find('>')?;
            let key = after[..key_end].trim().to_string();
            let value_text = &after[key_end + 1..];
            let value_end = value_text.find("</parameter>").unwrap_or(value_text.len());
            arguments.insert(
                key.clone(),
                self.value_of(&name, &key, &value_text[..value_end]),
            );
            rest = &value_text[value_end..];
        }
        Some(Piece::Call {
            name,
            arguments: Value::Object(arguments).to_string(),
        })
    }

    /// `<invoke name="x"><parameter name="k">v</parameter></invoke>`（可带 `antml:` 前缀）。
    fn parse_invokes(&self, block: &str) -> Vec<Piece> {
        let mut out = Vec::new();
        let mut rest = block;
        while let Some(at) = rest.find("invoke name=") {
            let after = &rest[at + "invoke name=".len()..];
            let Some(name) = quoted(after) else {
                break;
            };
            let Some(open_end) = after.find('>') else {
                break;
            };
            let body = &after[open_end + 1..];
            let close = body.find("invoke>").unwrap_or(body.len());
            let inner = &body[..close];
            if self.tools.contains_key(&name) {
                let mut arguments = serde_json::Map::new();
                let mut params = inner;
                while let Some(at) = params.find("parameter name=") {
                    let after = &params[at + "parameter name=".len()..];
                    let Some(key) = quoted(after) else {
                        break;
                    };
                    let Some(value_start) = after.find('>') else {
                        break;
                    };
                    let value_text = &after[value_start + 1..];
                    let value_end = value_text.find("</").unwrap_or(value_text.len());
                    arguments.insert(
                        key.clone(),
                        self.value_of(&name, &key, &value_text[..value_end]),
                    );
                    params = &value_text[value_end..];
                }
                out.push(Piece::Call {
                    name,
                    arguments: Value::Object(arguments).to_string(),
                });
            }
            rest = &body[close.min(body.len())..];
            if close >= body.len() {
                break;
            }
        }
        out
    }
}

/// `"x" …` → x。
fn quoted(text: &str) -> Option<String> {
    let text = text.trim_start();
    let inner = text.strip_prefix('"')?;
    let end = inner.find('"')?;
    Some(inner[..end].to_string())
}

/// Qoder 的 SSE：每行一个信封，`body` 是 OpenAI 分片的 JSON 字符串。
struct QoderSse {
    relay: ChatRelay,
    splitter: Splitter,
    vendor: &'static str,
}

impl QoderSse {
    fn apply(&mut self, pieces: Vec<Piece>) -> Vec<u8> {
        let mut out = Vec::new();
        for piece in pieces {
            match piece {
                Piece::Text(text) => out.extend(self.relay.text(&text)),
                Piece::Call { name, arguments } => {
                    out.extend(self.relay.whole_tool_call(&name, &arguments))
                }
            }
        }
        out
    }

    fn close(&mut self) -> anyhow::Result<Vec<u8>> {
        let pieces = self.splitter.finish()?;
        let mut out = self.apply(pieces);
        if self.relay.has_nameless_tool() {
            return Err(fail(502, format!("{} 返回的工具调用缺少名字", self.vendor)));
        }
        if !self.relay.said() {
            return Err(fail(502, format!("{} 返回了空回复", self.vendor)));
        }
        out.extend(self.relay.finish());
        Ok(out)
    }
}

impl SseHandler for QoderSse {
    fn event(&mut self, _event: &str, data: &str) -> anyhow::Result<Vec<u8>> {
        let data = data.trim();
        if data.is_empty() || self.relay.sse.is_finished() {
            return Ok(Vec::new());
        }
        let envelope: Value = serde_json::from_str(data)
            .map_err(|_| fail(502, format!("{} 返回了无法解析的数据", self.vendor)))?;
        if let Some(code) =
            b_shared::u64_at(&envelope, &["statusCodeValue"]).filter(|code| *code != 200)
        {
            let body = match envelope.get("body") {
                Some(Value::String(text)) => text.clone(),
                Some(other) => other.to_string(),
                None => data.to_string(),
            };
            return Err(failure(self.vendor, code as u16, &body));
        }
        let chunk = match envelope.get("body") {
            Some(Value::String(text)) if text.trim() == "[DONE]" => return self.close(),
            Some(Value::String(text)) => serde_json::from_str::<Value>(text)
                .map_err(|_| fail(502, format!("{} 返回了无法解析的分片", self.vendor)))?,
            Some(other @ Value::Object(_)) => other.clone(),
            _ => return Ok(Vec::new()),
        };
        let mut split_error = None;
        let splitter = &mut self.splitter;
        let out = self
            .relay
            .chunk_with(&chunk, |relay, text| match splitter.push(text) {
                Ok(pieces) => {
                    let mut bytes = Vec::new();
                    for piece in pieces {
                        match piece {
                            Piece::Text(text) => bytes.extend(relay.text(&text)),
                            Piece::Call { name, arguments } => {
                                bytes.extend(relay.whole_tool_call(&name, &arguments))
                            }
                        }
                    }
                    bytes
                }
                Err(error) => {
                    split_error = Some(error);
                    Vec::new()
                }
            })?;
        if let Some(error) = split_error {
            return Err(error);
        }
        Ok(out)
    }

    fn end(&mut self) -> anyhow::Result<Vec<u8>> {
        if self.relay.sse.is_finished() {
            return Ok(Vec::new());
        }
        self.close()
    }
}

// ---------------------------------------------------------------------------
// Subscription
// ---------------------------------------------------------------------------

#[async_trait::async_trait]
impl Subscription for Qoder {
    fn id(&self) -> &'static str {
        self.id
    }

    fn name(&self) -> &'static str {
        self.name
    }

    fn login_methods(&self) -> &'static [LoginMethod] {
        &[LoginMethod::DeviceCode, LoginMethod::Paste]
    }

    fn note(&self) -> &'static str {
        self.note
    }

    async fn begin_login(
        &self,
        _http: &reqwest::Client,
        method: LoginMethod,
        input: &str,
    ) -> anyhow::Result<LoginStart> {
        let machine_id = uuid::Uuid::new_v4().to_string();
        match method {
            LoginMethod::DeviceCode | LoginMethod::Browser => {
                let url_safe = base64::engine::general_purpose::URL_SAFE_NO_PAD;
                let verifier = url_safe.encode(b_shared::random_bytes(64));
                let challenge = {
                    use sha2::{Digest, Sha256};
                    url_safe.encode(Sha256::digest(verifier.as_bytes()))
                };
                let nonce = uuid::Uuid::new_v4().to_string();
                let url = format!(
                    "{}/device/selectAccounts?{}",
                    self.web,
                    oauth::query(&[
                        ("challenge", &challenge),
                        ("challenge_method", "S256"),
                        ("nonce", &nonce),
                        ("machine_id", &machine_id),
                        ("client_id", self.client_id),
                    ])
                );
                Ok(LoginStart {
                    open_url: Some(url),
                    user_code: None,
                    message: format!("在浏览器里选择 {} 账号并授权。", self.name),
                    state: json!({
                        "method": "device",
                        "verifier": verifier,
                        "nonce": nonce,
                        "machine_id": machine_id,
                    }),
                })
            }
            LoginMethod::Paste => {
                let token = input.trim();
                if token.is_empty() {
                    anyhow::bail!("请粘贴 {} 的个人访问令牌", self.name);
                }
                Ok(LoginStart {
                    message: "正在用个人访问令牌登录。".into(),
                    state: json!({ "method": "pat", "pat": token, "machine_id": machine_id }),
                    ..LoginStart::default()
                })
            }
            LoginMethod::ImportLocal => anyhow::bail!("{} 不支持导入本机登录", self.name),
        }
    }

    async fn finish_login(
        &self,
        http: &reqwest::Client,
        start: &LoginStart,
    ) -> anyhow::Result<LoginDone> {
        let state = &start.state;
        let machine_id = state
            .get("machine_id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        if state.get("method").and_then(Value::as_str) == Some("pat") {
            let pat = state.get("pat").and_then(Value::as_str).unwrap_or_default();
            let (status, answer) = openapi(
                http,
                &format!("{}/api/v1/jobToken/exchange", self.openapi),
                None,
                Some(&json!({ "personal_token": pat })),
            )
            .await?;
            if matches!(status, 400 | 401 | 403) {
                anyhow::bail!("个人访问令牌无效或已过期");
            }
            let access = b_shared::str_at(&answer, &["token", "device_token", "access_token"])
                .context("没有换到 job token")?
                .to_string();
            let (uid, email, name, account) = self.userinfo(http, &access).await?;
            return Ok(LoginDone {
                credentials: json!({
                    "access": access,
                    "refresh": b_shared::str_at(&answer, &["refresh_token", "refreshToken"]),
                    "expires_ms": job_expiry(&answer),
                    "uid": uid,
                    "email": email,
                    "name": name,
                    "machine_id": machine_id,
                    "pat": pat,
                }),
                account,
            });
        }
        let poll_url = format!(
            "{}/api/v1/deviceToken/poll?{}",
            self.openapi,
            oauth::query(&[
                (
                    "nonce",
                    state
                        .get("nonce")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                ),
                (
                    "verifier",
                    state
                        .get("verifier")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                ),
                ("challenge_method", "S256"),
            ])
        );
        let device = oauth::poll_until(Duration::from_secs(2), || {
            let http = http.clone();
            let poll_url = poll_url.clone();
            async move {
                Ok(match openapi(&http, &poll_url, None, None).await {
                    Ok((200, value)) if b_shared::str_at(&value, &["token"]).is_some() => {
                        Some(value)
                    }
                    _ => None,
                })
            }
        })
        .await?;
        let device_token = b_shared::str_at(&device, &["token"])
            .unwrap_or_default()
            .to_string();
        let device_refresh = b_shared::str_at(&device, &["refresh_token"])
            .unwrap_or_default()
            .to_string();
        let device_expires = device_expiry(&device);
        let (status, job) = openapi(
            http,
            &format!("{}/api/v1/me/jobToken", self.openapi),
            Some(&device_token),
            Some(&json!({ "clientId": self.client_id })),
        )
        .await?;
        let job_token = b_shared::str_at(&job, &["token"]).map(str::to_string);
        let (access, refresh, expires, device_chat) = match job_token {
            Some(token) if status == 200 => (
                token,
                b_shared::str_at(&job, &["refresh_token"])
                    .unwrap_or_default()
                    .to_string(),
                job_expiry(&job),
                false,
            ),
            _ if self.device_fallback && (400..500).contains(&status) => (
                device_token.clone(),
                device_refresh.clone(),
                device_expires,
                true,
            ),
            _ => anyhow::bail!("{} 换取聊天凭据失败（{status}）", self.name),
        };
        let (info_uid, email, name, account) = self.userinfo(http, &device_token).await?;
        let uid = b_shared::str_at(&device, &["user_id"])
            .map(str::to_string)
            .unwrap_or(info_uid);
        Ok(LoginDone {
            credentials: json!({
                "access": access,
                "refresh": refresh,
                "expires_ms": expires,
                "uid": uid,
                "email": email,
                "name": if name.is_empty() { b_shared::str_at(&device, &["user_name"]).unwrap_or_default().to_string() } else { name },
                "machine_id": machine_id,
                "device_token": device_token,
                "device_refresh": device_refresh,
                "device_expires_ms": device_expires,
                "device_chat": device_chat,
            }),
            account,
        })
    }

    async fn refresh(
        &self,
        http: &reqwest::Client,
        credentials: &Value,
    ) -> anyhow::Result<Option<Value>> {
        if !oauth::expiring(credentials, "expires_ms", REFRESH_AHEAD_MS) {
            return Ok(None);
        }
        let expires = credentials
            .get("expires_ms")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        let device_chat = credentials.get("device_chat").and_then(Value::as_bool) == Some(true);
        let refresh = b_shared::str_at(credentials, &["refresh"]).unwrap_or_default();
        let mut fresh = credentials.clone();
        let path = if device_chat {
            "/api/v1/deviceToken/refresh"
        } else {
            "/api/v1/jobToken/refresh"
        };
        let result = if refresh.is_empty() {
            Ok((401, Value::Null))
        } else {
            openapi(
                http,
                &format!("{}{path}", self.openapi),
                None,
                Some(&json!({ "refresh_token": refresh })),
            )
            .await
        };
        match result {
            Ok((200, value)) => {
                let token = b_shared::str_at(&value, &["token", "device_token"]);
                let rotated = b_shared::str_at(&value, &["refresh_token"]);
                let (Some(token), Some(rotated)) = (token, rotated) else {
                    return Err(fail(502, format!("{} 刷新登录的答复不完整", self.name)));
                };
                let expiry = if device_chat {
                    device_expiry(&value)
                } else {
                    job_expiry(&value)
                };
                fresh["access"] = json!(token);
                fresh["refresh"] = json!(rotated);
                fresh["expires_ms"] = json!(expiry);
                if device_chat {
                    fresh["device_token"] = json!(token);
                    fresh["device_refresh"] = json!(rotated);
                    fresh["device_expires_ms"] = json!(expiry);
                }
                Ok(Some(fresh))
            }
            Ok((401 | 403, _)) => {
                // 有个人访问令牌时重新换一次。
                if let Some(pat) = b_shared::str_at(credentials, &["pat"]) {
                    let (status, answer) = openapi(
                        http,
                        &format!("{}/api/v1/jobToken/exchange", self.openapi),
                        None,
                        Some(&json!({ "personal_token": pat })),
                    )
                    .await?;
                    if let Some(token) =
                        b_shared::str_at(&answer, &["token", "device_token", "access_token"])
                            .filter(|_| status == 200)
                    {
                        fresh["access"] = json!(token);
                        fresh["refresh"] = json!(b_shared::str_at(
                            &answer,
                            &["refresh_token", "refreshToken"]
                        ));
                        fresh["expires_ms"] = json!(job_expiry(&answer));
                        return Ok(Some(fresh));
                    }
                }
                Err(fail(401, format!("{} 登录已过期，请重新登录", self.name)))
            }
            // 刷新服务暂时不可用但 token 还没过期：先接着用。
            _ if expires > oauth::now_ms() => Ok(None),
            Ok((status, _)) => Err(fail(502, format!("{} 刷新登录失败（{status}）", self.name))),
            Err(error) => Err(fail(502, format!("{} 刷新登录失败：{error:#}", self.name))),
        }
    }

    async fn models(
        &self,
        http: &reqwest::Client,
        credentials: &Value,
    ) -> anyhow::Result<Vec<String>> {
        match self.listing(http, credentials, false).await {
            Ok(entries) if !entries.is_empty() => Ok(entries
                .iter()
                .filter_map(|entry| entry.get("key").and_then(Value::as_str).map(str::to_string))
                .collect()),
            Err(error)
                if error
                    .downcast_ref::<crate::gateway::subscriptions::SubscriptionError>()
                    .is_some_and(|error| error.status == 401) =>
            {
                Err(error)
            }
            _ => Ok(self
                .default_models
                .iter()
                .map(|model| model.to_string())
                .collect()),
        }
    }

    async fn chat(
        &self,
        http: &reqwest::Client,
        credentials: &Value,
        request: Value,
    ) -> anyhow::Result<ChatStream> {
        let model = request
            .get("model")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let find = |entries: &[Value]| {
            entries
                .iter()
                .find(|entry| entry.get("key").and_then(Value::as_str) == Some(model.as_str()))
                .cloned()
        };
        let entry = match find(&self.listing(http, credentials, false).await?) {
            Some(entry) => entry,
            None => find(&self.listing(http, credentials, true).await?)
                .ok_or_else(|| fail(400, format!("{} 账号里没有模型 {model}", self.name)))?,
        };
        let body = encode_body(&qoder_body(&request, &entry).to_string());
        let user = CosyUser::from_credentials(credentials)?;
        let mut headers = cosy_headers(&user, &body, CHAT_PATH, now_secs())?;
        headers.retain(|(key, _)| {
            !matches!(
                *key,
                "cosy-clientip" | "cosy-machinetoken" | "cosy-machinetype" | "accept"
            )
        });
        headers.push(("accept", "text/event-stream".into()));
        headers.push(("cache-control", "no-cache".into()));
        headers.push(("x-model-key", model.clone()));
        headers.push((
            "x-model-source",
            entry
                .get("source")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
        ));
        let url = format!(
            "{}?FetchKeys=llm_model_result&AgentId=agent_common&Encode=1",
            self.api_url(CHAT_PATH)
        );
        let mut builder = http.post(&url).body(body);
        for (key, value) in &headers {
            builder = builder.header(*key, value);
        }
        let response = builder
            .send()
            .await
            .map_err(|error| fail(502, format!("无法连接 {}：{error}", self.name)))?;
        let status = response.status().as_u16();
        if !(200..300).contains(&status) {
            let text = response.text().await.unwrap_or_default();
            return Err(failure(self.name, status, &text));
        }
        Ok(decode_stream(
            response,
            Sse::new(QoderSse {
                relay: ChatRelay::new(&model),
                splitter: Splitter::new(&request),
                vendor: self.name,
            }),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gateway::subscriptions::b_shared::run_decoder;
    use crate::gateway::subscriptions::chat::aggregate;

    fn status(error: &anyhow::Error) -> u16 {
        error
            .downcast_ref::<crate::gateway::subscriptions::SubscriptionError>()
            .unwrap()
            .status
    }

    #[test]
    fn md5_matches_vectors() {
        assert_eq!(md5_hex(b""), "d41d8cd98f00b204e9800998ecf8427e");
        assert_eq!(md5_hex(b"abc"), "900150983cd24fb0d6963f7d28e17f72");
        assert_eq!(
            md5_hex(b"The quick brown fox jumps over the lazy dog"),
            "9e107d9d372bb6826bd81d3542a419d6"
        );
        assert_eq!(md5_hex(&[b'a'; 64]), "014842d480b571495a4a0363793f7367");
    }

    #[test]
    fn aes_cbc_matches_nist_vector() {
        let hex = |text: &str| -> Vec<u8> {
            (0..text.len())
                .step_by(2)
                .map(|i| u8::from_str_radix(&text[i..i + 2], 16).unwrap())
                .collect()
        };
        let key: [u8; 16] = hex("2b7e151628aed2a6abf7158809cf4f3c").try_into().unwrap();
        let iv: [u8; 16] = hex("000102030405060708090a0b0c0d0e0f").try_into().unwrap();
        let out = aes128_cbc(&key, &iv, &hex("6bc1bee22e409f96e93d7e117393172a"));
        assert_eq!(
            &out[..16],
            hex("7649abac8119b246cee98e9b12e9197d").as_slice()
        );
        // 整块明文也要补一整块填充。
        assert_eq!(out.len(), 32);
    }

    #[test]
    fn rsa_math_and_key() {
        // 教科书例子：n = 3233, e = 17, m = 65 → c = 2790。
        let n = Big::from_be(&3233_u32.to_be_bytes());
        let e = Big::from_be(&17_u32.to_be_bytes());
        let m = Big::from_be(&65_u32.to_be_bytes());
        assert_eq!(m.mod_pow(&e, &n).to_be(2), 2790_u16.to_be_bytes().to_vec());
        let spki = base64::engine::general_purpose::STANDARD
            .decode(RSA_KEY)
            .unwrap();
        let (n, e) = rsa_public_key(&spki).unwrap();
        assert_eq!(n.bits(), 1024);
        assert_eq!(e, Big::from_be(&[1, 0, 1]));
        let cipher = rsa_encrypt(b"0123456789abcdef").unwrap();
        assert_eq!(cipher.len(), 128);
        assert!(!Big::from_be(&cipher).ge(&n));
        // 随机填充：两次结果不同。
        assert_ne!(cipher, rsa_encrypt(b"0123456789abcdef").unwrap());
    }

    #[test]
    fn body_codec_round_trips() {
        for text in ["", "a", "ab", "{\"model\":\"ultimate\",\"x\":\"中文\"}"] {
            assert_eq!(decode_body(&encode_body(text)), text);
        }
        let encoded = encode_body("hello world");
        assert!(
            encoded
                .bytes()
                .all(|byte| ALPHABET.contains(&byte) || byte == b'$')
        );
        assert_eq!(swap_thirds(&swap_thirds("abcdefghij")), "abcdefghij");
    }

    #[test]
    fn cosy_headers_are_signed() {
        let creds = json!({ "access": "t", "uid": "u1", "machine_id": "m1", "email": "a@b.c" });
        let user = CosyUser::from_credentials(&creds).unwrap();
        let headers = cosy_headers(&user, "", "/algo/api/v2/model/list", 1_700_000_000).unwrap();
        let get = |key: &str| {
            headers
                .iter()
                .find(|(name, _)| *name == key)
                .map(|(_, value)| value.clone())
                .unwrap()
        };
        let auth = get("authorization");
        let parts: Vec<&str> = auth.trim_start_matches("Bearer COSY.").split('.').collect();
        assert_eq!(parts.len(), 2);
        let expected = md5_hex(
            format!(
                "{}\n{}\n1700000000\n\n/api/v2/model/list",
                parts[0],
                get("cosy-key")
            )
            .as_bytes(),
        );
        assert_eq!(parts[1], expected);
        let payload: Value = serde_json::from_slice(
            &base64::engine::general_purpose::STANDARD
                .decode(parts[0])
                .unwrap(),
        )
        .unwrap();
        assert_eq!(payload["cosyVersion"], COSY_VERSION);
        assert_eq!(get("cosy-user"), "u1");
        assert_eq!(get("cosy-date"), "1700000000");
        assert!(CosyUser::from_credentials(&json!({ "access": "t" })).is_err());
    }

    fn entry() -> Value {
        json!({
            "key": "ultimate", "source": "system", "enable": true, "max_input_tokens": 180000,
            "thinking_config": { "enabled": { "efforts": { "low": {}, "high": { "is_default": true }, "max": {} } }, "disabled": {} },
            "context_config": { "a": { "token_count": 200000, "is_default": true }, "b": { "token_count": 1000000 } }
        })
    }

    #[test]
    fn picks_effort_and_window() {
        let entry = entry();
        assert_eq!(effort_for(None, &entry), (true, "high".to_string()));
        assert_eq!(effort_for(Some("none"), &entry), (false, String::new()));
        assert_eq!(
            effort_for(Some("medium"), &entry),
            (true, "high".to_string())
        );
        assert_eq!(effort_for(Some("xhigh"), &entry), (true, "max".to_string()));
        assert_eq!(effort_for(Some("ultra"), &entry), (true, "max".to_string()));
        let always =
            json!({ "thinking_config": { "enabled": { "efforts": { "low": {}, "high": {} } } } });
        assert_eq!(effort_for(Some("none"), &always), (true, "low".to_string()));
        assert_eq!(effort_for(Some("high"), &json!({})), (false, String::new()));
        assert_eq!(window_for(&entry, 1000), Some(200_000));
        assert_eq!(window_for(&entry, 500_000), Some(1_000_000));
        assert_eq!(window_for(&entry, 5_000_000), Some(1_000_000));
        assert_eq!(window_for(&json!({ "max_input_tokens": 9 }), 1), Some(9));
    }

    #[test]
    fn builds_body_with_tools_and_tool_results() {
        let request = json!({
            "model": "ultimate",
            "reasoning_effort": "low",
            "tool_choice": "required",
            "messages": [
                { "role": "system", "content": "规则" },
                { "role": "user", "content": "列出文件" },
                { "role": "assistant", "content": "好", "reasoning_content": "想", "tool_calls": [
                    { "id": "c1", "type": "function", "function": { "name": "ls", "arguments": "{}" } },
                    { "type": "function", "function": { "name": "cat", "arguments": "{\"p\":1}" } }
                ]},
                { "role": "tool", "tool_call_id": "zzz", "content": "a b" },
                { "role": "user", "content": [{ "type": "text", "text": "继续" }] }
            ],
            "tools": [{ "type": "function", "function": { "name": "ls", "parameters": { "oneOf": [{ "type": "object" }] } } }]
        });
        let body = qoder_body(&request, &entry());
        assert_eq!(body["model_config"]["key"], "ultimate");
        assert_eq!(body["request_id"], body["chat_record_id"]);
        assert_eq!(body["business"]["stage"], "start");
        assert_eq!(body["parameters"]["reasoning_effort"], "low");
        assert_eq!(body["parameters"]["context_length"], 200_000);
        let system = body["system"][0]["text"].as_str().unwrap();
        assert!(system.starts_with(SYSTEM_PREFIX));
        assert!(system.contains("规则"));
        assert!(system.ends_with("You must call an available function in this response."));
        assert_eq!(body["tools"][0]["function"]["parameters"]["type"], "object");
        let messages = body["messages"].as_array().unwrap();
        assert_eq!(messages[0]["role"], "system");
        assert_eq!(messages[1]["contents"], messages[1]["content"]);
        let assistant = &messages[2];
        assert_eq!(assistant["content"], "好");
        assert_eq!(assistant["reasoning_content"], "想");
        let generated = assistant["tool_calls"][1]["id"].as_str().unwrap();
        assert!(generated.starts_with("call_") && generated.len() == 37);
        // 对不上的结果分给第一个没回答的调用；剩下的补「被打断」。
        assert_eq!(messages[3]["tool_call_id"], "c1");
        assert_eq!(messages[4]["tool_call_id"], generated);
        assert_eq!(messages[4]["content"], INTERRUPTED);
        assert_eq!(
            messages[5]["content"][0]["cache_control"]["type"],
            "ephemeral"
        );
        // 同一轮的任务 id 和会话 id 稳定。
        let again = qoder_body(&request, &entry());
        assert_eq!(again["request_set_id"], body["request_set_id"]);
        assert_eq!(again["session_id"], body["session_id"]);
        assert_ne!(again["request_id"], body["request_id"]);
    }

    #[test]
    fn scene_models_merge_and_filter() {
        let listing = json!({
            "app": [{ "key": "ultimate", "enable": true }, { "key": "auto", "enable": true }, { "key": "off", "enable": false }],
            "chat": [{ "key": "ultimate", "enable": true, "x": 1 }, { "key": "performance", "enable": true }]
        });
        let keys: Vec<String> = scene_models(&listing)
            .iter()
            .map(|entry| entry["key"].as_str().unwrap().to_string())
            .collect();
        assert_eq!(keys, vec!["ultimate", "performance"]);
        assert!(scene_models(&listing)[0].get("x").is_none());
    }

    #[test]
    fn maps_errors() {
        assert_eq!(
            status(&failure("Q", 200, r#"{"code":"10605","message":"queued"}"#)),
            503
        );
        assert_eq!(
            status(&failure(
                "Q",
                403,
                r#"{"code":"","message":"{\"code\":\"10605\"}"}"#
            )),
            503
        );
        assert_eq!(
            status(&failure(
                "Q",
                400,
                r#"{"code":"113","message":"no credits"}"#
            )),
            429
        );
        assert_eq!(
            status(&failure("Q", 400, r#"{"code":"100400","message":"bad"}"#)),
            400
        );
        assert_eq!(
            status(&failure(
                "Q",
                400,
                r#"{"message":"Error in upstream response"}"#
            )),
            502
        );
        assert_eq!(status(&failure("Q", 401, "")), 401);
        assert_eq!(status(&failure("Q", 429, "")), 429);
    }

    #[test]
    fn expiry_rules() {
        let now = oauth::now_ms();
        let job = job_expiry(&json!({ "expires_in": 60_000 }));
        assert!(job >= now + 59_000 && job <= now + 61_000);
        let at = job_expiry(&json!({ "expire_time": 4_102_444_800_u64 }));
        assert_eq!(at, 4_102_444_800_000);
        assert_eq!(
            job_expiry(&json!({ "expires_at": "2100-01-01T00:00:00Z" })),
            4_102_444_800_000
        );
        let device = device_expiry(&json!({ "expires_in": 60 }));
        assert!(device >= now + 59_000 && device <= now + 61_000);
        assert!(job_expiry(&json!({})) >= now + DAY_MS - 1000);
        assert_eq!(mask_phone("13800138000"), "138****8000");
        let stale = json!({ "expires_ms": now + 60_000 });
        assert!(oauth::expiring(&stale, "expires_ms", REFRESH_AHEAD_MS));
    }

    fn tools_request() -> Value {
        json!({ "tools": [
            { "type": "function", "function": { "name": "read", "parameters": { "type": "object", "properties": { "path": { "type": "string" }, "limit": { "type": "number" } } } } }
        ]})
    }

    #[test]
    fn splitter_extracts_tool_calls_from_text() {
        let mut splitter = Splitter::new(&tools_request());
        let mut pieces = Vec::new();
        for part in [
            "好的<tool",
            "_call>{\"name\":\"read\",\"argu",
            "ments\":{\"path\":\"a\"}}</tool_call>完",
        ] {
            pieces.extend(splitter.push(part).unwrap());
        }
        pieces.extend(splitter.finish().unwrap());
        assert_eq!(
            pieces,
            vec![
                Piece::Text("好的".into()),
                Piece::Call {
                    name: "read".into(),
                    arguments: "{\"path\":\"a\"}".into()
                },
                Piece::Text("完".into()),
            ]
        );

        let mut splitter = Splitter::new(&tools_request());
        let pieces = splitter
            .push("<function=read>\n<parameter=path>\n007\n</parameter>\n<parameter=limit>5</parameter>\n</function>")
            .unwrap();
        let Piece::Call { name, arguments } = &pieces[0] else {
            panic!("应当切出工具调用：{pieces:?}");
        };
        assert_eq!(name, "read");
        let arguments: Value = serde_json::from_str(arguments).unwrap();
        assert_eq!(arguments, json!({ "path": "007", "limit": 5 }));

        let mut splitter = Splitter::new(&tools_request());
        let pieces = splitter
            .push("<function_calls><invoke name=\"read\"><parameter name=\"path\">b</parameter></invoke></function_calls>")
            .unwrap();
        assert_eq!(
            pieces,
            vec![Piece::Call {
                name: "read".into(),
                arguments: "{\"path\":\"b\"}".into()
            }]
        );
    }

    #[test]
    fn splitter_leaves_code_and_unknown_tools_alone() {
        let mut splitter = Splitter::new(&tools_request());
        let mut pieces = splitter
            .push("```\n<tool_call>{\"name\":\"read\"}</tool_call>\n```")
            .unwrap();
        pieces.extend(splitter.finish().unwrap());
        assert!(pieces.iter().all(|piece| matches!(piece, Piece::Text(_))));

        let mut splitter = Splitter::new(&tools_request());
        let pieces = splitter
            .push("<function=rm><parameter=x>1</parameter></function>")
            .unwrap();
        assert!(matches!(&pieces[0], Piece::Text(text) if text.contains("rm")));

        // 没声明工具时裸标签不识别。
        let mut splitter = Splitter::new(&json!({}));
        let pieces = splitter.push("用 <function=read> 读").unwrap();
        assert_eq!(pieces, vec![Piece::Text("用 <function=read> 读".into())]);

        // 截断的调用是错误；JSON 完整但缺结束标签的算有效。
        let mut splitter = Splitter::new(&tools_request());
        splitter.push("<tool_call>{\"name\":\"read\"").unwrap();
        assert!(splitter.finish().is_err());
        let mut splitter = Splitter::new(&tools_request());
        splitter
            .push("<tool_call>{\"name\":\"read\",\"arguments\":{}}")
            .unwrap();
        assert_eq!(splitter.finish().unwrap().len(), 1);
        let mut splitter = Splitter::new(&tools_request());
        assert!(splitter.push("<tool_call>{\"name\":}</tool_call>").is_err());
    }

    fn envelope(chunk: &Value) -> String {
        format!(
            "data:{}\n\n",
            json!({ "statusCodeValue": 200, "body": chunk.to_string() })
        )
    }

    #[test]
    fn decodes_envelopes() {
        let events = vec![
            envelope(&json!({ "choices": [{ "delta": { "reasoning_content": "想" } }] })),
            envelope(
                &json!({ "choices": [{ "delta": { "content": "看<tool_call>{\"name\":\"read\",\"arguments\":{\"path\":\"x\"}}</tool_call>" } }] }),
            ),
            envelope(
                &json!({ "choices": [{ "delta": { "tool_calls": [{ "index": 0, "id": "n1", "function": { "name": "read", "arguments": "{}" } }] } }] }),
            ),
            envelope(&json!({ "choices": [{ "delta": {}, "finish_reason": "stop" }] })),
            envelope(&json!({ "usage": { "prompt_tokens": 9, "completion_tokens": 3 } })),
            format!(
                "data:{}\n\n",
                json!({ "statusCodeValue": 200, "body": "[DONE]" })
            ),
        ];
        let parts: Vec<&[u8]> = events.iter().map(|event| event.as_bytes()).collect();
        let handler = QoderSse {
            relay: ChatRelay::new("ultimate"),
            splitter: Splitter::new(&tools_request()),
            vendor: "Qoder",
        };
        let (text, error) = run_decoder(Sse::new(handler), &parts);
        assert!(error.is_none(), "{error:?}");
        let completion = aggregate(&text, "ultimate").unwrap();
        let message = &completion["choices"][0]["message"];
        assert_eq!(message["reasoning_content"], "想");
        assert_eq!(message["content"], "看");
        assert_eq!(message["tool_calls"].as_array().unwrap().len(), 2);
        assert_eq!(
            message["tool_calls"][0]["function"]["arguments"],
            "{\"path\":\"x\"}"
        );
        assert_eq!(message["tool_calls"][1]["id"], "n1");
        assert_eq!(completion["choices"][0]["finish_reason"], "tool_calls");
        assert_eq!(completion["usage"]["prompt_tokens"], 9);

        let queued = format!(
            "data:{}\n\n",
            json!({ "statusCodeValue": 403, "body": "{\"code\":\"10605\",\"message\":\"busy\"}" })
        );
        let handler = QoderSse {
            relay: ChatRelay::new("m"),
            splitter: Splitter::default(),
            vendor: "Qoder",
        };
        let (_, error) = run_decoder(Sse::new(handler), &[queued.as_bytes()]);
        assert_eq!(status(&error.unwrap()), 503);
    }
}
