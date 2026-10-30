//! Zed 的托管模型（Zed Pro / 试用 / 学生 / Business）。
//!
//! - 登录：Zed 自己的浏览器登录——本机生成 RSA-2048 密钥，浏览器打开
//!   `zed.dev/native_app_signin`（带本机端口和公钥），登录后 zed.dev 把浏览器送回本机端口，
//!   带上 user_id 和用公钥加密的 access token（RSA-OAEP-SHA256，旧格式 PKCS#1 v1.5）。
//!   没有 RSA 库，这里手写最小的大数模幂 + OAEP 解码。也可以导入 macOS 钥匙串里 Zed 保存的登录，
//!   或粘贴 `<user_id> <access_token>`。
//! - 请求：账号凭据（`Authorization: <user_id> <token>`）按组织换短期的模型 token
//!   （`/client/llm_tokens`，约一小时），再 `POST /completions`：`{provider, model, provider_request}`，
//!   `provider_request` 是该模型所属厂商自己的请求格式（Anthropic Messages / OpenAI Responses /
//!   xAI Chat / Gemini generateContent）；答复是逐行 JSON（`{"event": …}` / `{"status": …}`）。

use std::collections::{HashMap, HashSet};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use anyhow::{Context, bail};
use base64::Engine;
use futures_util::StreamExt;
use serde_json::{Value, json};

use crate::gateway::subscriptions::chat::{ChatSse, Usage};
use crate::gateway::subscriptions::editor_kit::{
    self, AnthropicEvents, ChatChunks, Emitter, LineReader, arguments_object, content_text,
    reasoning_effort, text_of,
};
use crate::gateway::subscriptions::oauth::{LOGIN_TIMEOUT, now_ms, query};
use crate::gateway::subscriptions::{
    ChatStream, LoginDone, LoginMethod, LoginStart, Subscription, fail,
};

const SITE: &str = "https://zed.dev";
const CLOUD: &str = "https://cloud.zed.dev";
const VERSION: &str = "1.23.0";
const SUCCEEDED: &str = "https://zed.dev/native_app_signin_succeeded";
/// 模型 token 按一小时算，提前 10 分钟换。
const LLM_TOKEN_TTL_MS: u64 = 50 * 60 * 1000;
const REFRESH_AHEAD_MS: u64 = 5 * 60 * 1000;

pub struct Zed;

pub static ZED: Zed = Zed;

/* —————————————— RSA —————————————— */

/// 最小的 RSA：大数（u64 小端）、Montgomery 模幂、素数生成、OAEP-SHA256 / PKCS#1 v1.5 解码。
/// 只用来解一次登录回调里的 token，不追求常数时间。
pub(crate) mod rsa {
    use std::cmp::Ordering;

    use anyhow::bail;
    use sha2::{Digest, Sha256};

    /// 无符号大数，u64 小端，去掉高位的 0。
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct Big(Vec<u64>);

    impl Big {
        fn trim(mut self) -> Self {
            while self.0.last() == Some(&0) {
                self.0.pop();
            }
            self
        }

        pub fn from_u64(value: u64) -> Self {
            Big(vec![value]).trim()
        }

        pub fn from_be(bytes: &[u8]) -> Self {
            let mut limbs = Vec::with_capacity(bytes.len() / 8 + 1);
            for chunk in bytes.rchunks(8) {
                let mut word = [0_u8; 8];
                word[8 - chunk.len()..].copy_from_slice(chunk);
                limbs.push(u64::from_be_bytes(word));
            }
            Big(limbs).trim()
        }

        /// 大端字节，左边补 0 到 `len`（`len` 为 0 时用最短长度）。
        pub fn to_be(&self, len: usize) -> Vec<u8> {
            let mut bytes: Vec<u8> = self
                .0
                .iter()
                .rev()
                .flat_map(|limb| limb.to_be_bytes())
                .collect();
            let first = bytes
                .iter()
                .position(|byte| *byte != 0)
                .unwrap_or(bytes.len());
            bytes.drain(..first);
            if bytes.len() < len {
                let mut padded = vec![0; len - bytes.len()];
                padded.extend(bytes);
                bytes = padded;
            }
            bytes
        }

        pub fn is_odd(&self) -> bool {
            self.0.first().is_some_and(|limb| limb & 1 == 1)
        }

        pub fn bits(&self) -> usize {
            match self.0.last() {
                Some(top) => self.0.len() * 64 - top.leading_zeros() as usize,
                None => 0,
            }
        }

        fn bit(&self, index: usize) -> bool {
            self.0
                .get(index / 64)
                .is_some_and(|limb| (limb >> (index % 64)) & 1 == 1)
        }

        pub fn add(&self, other: &Big) -> Big {
            let mut out = Vec::with_capacity(self.0.len().max(other.0.len()) + 1);
            let mut carry = 0_u128;
            for index in 0..self.0.len().max(other.0.len()) {
                let sum = u128::from(*self.0.get(index).unwrap_or(&0))
                    + u128::from(*other.0.get(index).unwrap_or(&0))
                    + carry;
                out.push(sum as u64);
                carry = sum >> 64;
            }
            out.push(carry as u64);
            Big(out).trim()
        }

        /// self - other（要求 self >= other）。
        pub fn sub(&self, other: &Big) -> Big {
            let mut out = Vec::with_capacity(self.0.len());
            let mut borrow = 0_i128;
            for index in 0..self.0.len() {
                let mut diff = i128::from(self.0[index])
                    - i128::from(*other.0.get(index).unwrap_or(&0))
                    - borrow;
                borrow = 0;
                if diff < 0 {
                    diff += 1_i128 << 64;
                    borrow = 1;
                }
                out.push(diff as u64);
            }
            Big(out).trim()
        }

        pub fn mul(&self, other: &Big) -> Big {
            let mut out = vec![0_u64; self.0.len() + other.0.len() + 1];
            for (i, a) in self.0.iter().enumerate() {
                let mut carry = 0_u128;
                for (j, b) in other.0.iter().enumerate() {
                    let current = u128::from(out[i + j]) + u128::from(*a) * u128::from(*b) + carry;
                    out[i + j] = current as u64;
                    carry = current >> 64;
                }
                let mut index = i + other.0.len();
                while carry > 0 {
                    let current = u128::from(out[index]) + carry;
                    out[index] = current as u64;
                    carry = current >> 64;
                    index += 1;
                }
            }
            Big(out).trim()
        }

        /// 除以小数：(商, 余数)。
        pub fn div_small(&self, divisor: u64) -> (Big, u64) {
            let mut out = vec![0_u64; self.0.len()];
            let mut rest = 0_u128;
            for index in (0..self.0.len()).rev() {
                let current = (rest << 64) | u128::from(self.0[index]);
                out[index] = (current / u128::from(divisor)) as u64;
                rest = current % u128::from(divisor);
            }
            (Big(out).trim(), rest as u64)
        }

        fn shl1(&self) -> Big {
            let mut out = Vec::with_capacity(self.0.len() + 1);
            let mut carry = 0;
            for limb in &self.0 {
                out.push((limb << 1) | carry);
                carry = limb >> 63;
            }
            out.push(carry);
            Big(out).trim()
        }
    }

    impl PartialOrd for Big {
        fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
            Some(self.cmp(other))
        }
    }

    impl Ord for Big {
        fn cmp(&self, other: &Self) -> Ordering {
            self.0
                .len()
                .cmp(&other.0.len())
                .then_with(|| self.0.iter().rev().cmp(other.0.iter().rev()))
        }
    }

    /// 奇数模数上的 Montgomery 乘法。
    struct Mont {
        n: Vec<u64>,
        n0: u64,
        r2: Big,
    }

    impl Mont {
        fn new(modulus: &Big) -> Self {
            let n = modulus.0.clone();
            // -n^{-1} mod 2^64（牛顿迭代）。
            let mut inverse = 1_u64;
            for _ in 0..6 {
                inverse = inverse.wrapping_mul(2_u64.wrapping_sub(n[0].wrapping_mul(inverse)));
            }
            // R^2 mod n：从 1 开始翻倍 2·64·k 次。
            let mut r2 = Big::from_u64(1);
            for _ in 0..(2 * 64 * n.len()) {
                r2 = r2.shl1();
                if r2 >= *modulus {
                    r2 = r2.sub(modulus);
                }
            }
            Mont {
                n,
                n0: inverse.wrapping_neg(),
                r2,
            }
        }

        /// CIOS 形式的 Montgomery 乘法：a·b·R⁻¹ mod n。按下标写更贴近算法原文。
        #[allow(clippy::needless_range_loop)]
        fn mul(&self, a: &[u64], b: &[u64]) -> Vec<u64> {
            let k = self.n.len();
            let mut t = vec![0_u64; k + 2];
            for i in 0..k {
                let ai = u128::from(*a.get(i).unwrap_or(&0));
                let mut carry = 0_u128;
                for j in 0..k {
                    let current =
                        u128::from(t[j]) + ai * u128::from(*b.get(j).unwrap_or(&0)) + carry;
                    t[j] = current as u64;
                    carry = current >> 64;
                }
                let current = u128::from(t[k]) + carry;
                t[k] = current as u64;
                t[k + 1] = (current >> 64) as u64;
                let m = u128::from(t[0].wrapping_mul(self.n0));
                let mut carry = (u128::from(t[0]) + m * u128::from(self.n[0])) >> 64;
                for j in 1..k {
                    let current = u128::from(t[j]) + m * u128::from(self.n[j]) + carry;
                    t[j - 1] = current as u64;
                    carry = current >> 64;
                }
                let current = u128::from(t[k]) + carry;
                t[k - 1] = current as u64;
                t[k] = t[k + 1] + (current >> 64) as u64;
                t[k + 1] = 0;
            }
            let mut out = Big(t[..=k].to_vec()).trim();
            let modulus = Big(self.n.clone());
            if out >= modulus {
                out = out.sub(&modulus);
            }
            let mut limbs = out.0;
            limbs.resize(k, 0);
            limbs
        }

        fn to_mont(&self, value: &Big) -> Vec<u64> {
            self.mul(&value.0, &self.r2.0)
        }

        fn leave_mont(&self, value: &[u64]) -> Big {
            Big(self.mul(value, &[1])).trim()
        }

        fn pow(&self, base: &Big, exponent: &Big) -> Big {
            let base = self.to_mont(base);
            let mut result = self.to_mont(&Big::from_u64(1));
            for index in (0..exponent.bits()).rev() {
                result = self.mul(&result, &result);
                if exponent.bit(index) {
                    result = self.mul(&result, &base);
                }
            }
            self.leave_mont(&result)
        }
    }

    /// base^exponent mod modulus（modulus 为奇数）。
    pub fn mod_pow(base: &Big, exponent: &Big, modulus: &Big) -> Big {
        Mont::new(modulus).pow(base, exponent)
    }

    const SMALL_PRIMES: usize = 2000;

    fn small_primes() -> &'static [u64] {
        static PRIMES: std::sync::OnceLock<Vec<u64>> = std::sync::OnceLock::new();
        PRIMES.get_or_init(|| {
            let mut sieve = vec![true; SMALL_PRIMES];
            let mut out = Vec::new();
            for number in 2..SMALL_PRIMES {
                if sieve[number] {
                    out.push(number as u64);
                    let mut multiple = number * number;
                    while multiple < SMALL_PRIMES {
                        sieve[multiple] = false;
                        multiple += number;
                    }
                }
            }
            out
        })
    }

    /// Miller–Rabin（先试除小素数）。
    pub fn probably_prime(
        candidate: &Big,
        rounds: usize,
        random: &mut dyn FnMut(usize) -> Vec<u8>,
    ) -> bool {
        for prime in small_primes() {
            let (_, rest) = candidate.div_small(*prime);
            if rest == 0 {
                return *candidate == Big::from_u64(*prime);
            }
        }
        let one = Big::from_u64(1);
        let minus_one = candidate.sub(&one);
        let mut d = minus_one.clone();
        let mut s = 0;
        while !d.is_odd() {
            d = d.div_small(2).0;
            s += 1;
        }
        let mont = Mont::new(candidate);
        let one_m = mont.to_mont(&one);
        let minus_one_m = mont.to_mont(&minus_one);
        let bytes = candidate.bits().div_ceil(8);
        'round: for _ in 0..rounds {
            let mut base = Big::from_be(&random(bytes.saturating_sub(1).max(1)));
            if base < Big::from_u64(2) {
                base = Big::from_u64(2);
            }
            let mut x = mont.to_mont(&mont.pow(&base, &d));
            if x == one_m || x == minus_one_m {
                continue;
            }
            for _ in 1..s {
                x = mont.mul(&x, &x);
                if x == minus_one_m {
                    continue 'round;
                }
            }
            return false;
        }
        true
    }

    /// 生成 `bits` 位的素数 p，且 gcd(e, p-1) = 1。
    fn random_prime(bits: usize, e: u64, random: &mut dyn FnMut(usize) -> Vec<u8>) -> Big {
        loop {
            let mut bytes = random(bits / 8);
            bytes[0] |= 0xC0;
            let last = bytes.len() - 1;
            bytes[last] |= 1;
            let candidate = Big::from_be(&bytes);
            if candidate.sub(&Big::from_u64(1)).div_small(e).1 == 0 {
                continue;
            }
            if probably_prime(&candidate, 8, random) {
                return candidate;
            }
        }
    }

    /// 小数的模逆（扩展欧几里得）。
    fn inverse_small(value: u64, modulus: u64) -> Option<u64> {
        let (mut old_r, mut r) = (i128::from(value), i128::from(modulus));
        let (mut old_s, mut s) = (1_i128, 0_i128);
        while r != 0 {
            let quotient = old_r / r;
            (old_r, r) = (r, old_r - quotient * r);
            (old_s, s) = (s, old_s - quotient * s);
        }
        if old_r != 1 {
            return None;
        }
        Some(old_s.rem_euclid(i128::from(modulus)) as u64)
    }

    #[derive(Clone)]
    pub struct PrivateKey {
        pub n: Big,
        pub e: Big,
        pub d: Big,
    }

    impl std::fmt::Debug for PrivateKey {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "PrivateKey({} bits)", self.n.bits())
        }
    }

    impl PrivateKey {
        /// 生成 `bits` 位的密钥（e = 65537）。
        pub fn generate(bits: usize, random: &mut dyn FnMut(usize) -> Vec<u8>) -> PrivateKey {
            const E: u64 = 65_537;
            loop {
                let p = random_prime(bits / 2, E, random);
                let q = random_prime(bits / 2, E, random);
                if p == q {
                    continue;
                }
                let n = p.mul(&q);
                if n.bits() != bits {
                    continue;
                }
                let one = Big::from_u64(1);
                let phi = p.sub(&one).mul(&q.sub(&one));
                // d = (1 + k·φ) / e，k ≡ -φ^{-1} (mod e)。
                let Some(inverse) = inverse_small(phi.div_small(E).1, E) else {
                    continue;
                };
                let k = (E - inverse) % E;
                let (d, rest) = one.add(&phi.mul(&Big::from_u64(k))).div_small(E);
                if rest != 0 {
                    continue;
                }
                return PrivateKey {
                    n,
                    e: Big::from_u64(E),
                    d,
                };
            }
        }

        pub fn size(&self) -> usize {
            self.n.bits().div_ceil(8)
        }

        /// PKCS#1 RSAPublicKey 的 DER。
        pub fn public_der(&self) -> Vec<u8> {
            fn integer(value: &Big) -> Vec<u8> {
                let mut bytes = value.to_be(0);
                if bytes.is_empty() || bytes[0] & 0x80 != 0 {
                    bytes.insert(0, 0);
                }
                tlv(0x02, &bytes)
            }
            fn tlv(tag: u8, body: &[u8]) -> Vec<u8> {
                let mut out = vec![tag];
                let length = body.len();
                if length < 0x80 {
                    out.push(length as u8);
                } else {
                    let bytes: Vec<u8> = length
                        .to_be_bytes()
                        .iter()
                        .copied()
                        .skip_while(|byte| *byte == 0)
                        .collect();
                    out.push(0x80 | bytes.len() as u8);
                    out.extend(bytes);
                }
                out.extend_from_slice(body);
                out
            }
            let mut body = integer(&self.n);
            body.extend(integer(&self.e));
            tlv(0x30, &body)
        }

        /// 原始 RSA：c^d mod n，按密钥长度补齐。
        pub fn raw_decrypt(&self, ciphertext: &[u8]) -> anyhow::Result<Vec<u8>> {
            let c = Big::from_be(ciphertext);
            if c >= self.n {
                bail!("密文比模数还大");
            }
            Ok(mod_pow(&c, &self.d, &self.n).to_be(self.size()))
        }

        /// 先按 OAEP-SHA256 解，失败再按 PKCS#1 v1.5（Zed 的旧格式）。
        pub fn decrypt(&self, ciphertext: &[u8]) -> anyhow::Result<Vec<u8>> {
            let message = self.raw_decrypt(ciphertext)?;
            oaep_decode(&message).or_else(|_| pkcs1_decode(&message))
        }

        /// 加密（测试用）：OAEP-SHA256。
        #[cfg(test)]
        pub fn encrypt_oaep(&self, plaintext: &[u8], seed: &[u8; 32]) -> Vec<u8> {
            let k = self.size();
            let mut db = Sha256::digest(b"").to_vec();
            db.extend(vec![0_u8; k - plaintext.len() - 2 * 32 - 2]);
            db.push(1);
            db.extend_from_slice(plaintext);
            let db_mask = mgf1(seed, db.len());
            let masked_db: Vec<u8> = db.iter().zip(&db_mask).map(|(a, b)| a ^ b).collect();
            let seed_mask = mgf1(&masked_db, 32);
            let masked_seed: Vec<u8> = seed.iter().zip(&seed_mask).map(|(a, b)| a ^ b).collect();
            let mut message = vec![0_u8];
            message.extend(masked_seed);
            message.extend(masked_db);
            mod_pow(&Big::from_be(&message), &self.e, &self.n).to_be(k)
        }
    }

    fn mgf1(seed: &[u8], length: usize) -> Vec<u8> {
        let mut out = Vec::with_capacity(length + 32);
        let mut counter = 0_u32;
        while out.len() < length {
            let mut hasher = Sha256::new();
            hasher.update(seed);
            hasher.update(counter.to_be_bytes());
            out.extend_from_slice(&hasher.finalize());
            counter += 1;
        }
        out.truncate(length);
        out
    }

    /// RFC 8017 §7.1.2（SHA-256，空 label）。
    pub fn oaep_decode(message: &[u8]) -> anyhow::Result<Vec<u8>> {
        const H: usize = 32;
        if message.len() < 2 * H + 2 || message[0] != 0 {
            bail!("不是 OAEP 编码");
        }
        let (masked_seed, masked_db) = message[1..].split_at(H);
        let seed: Vec<u8> = masked_seed
            .iter()
            .zip(mgf1(masked_db, H))
            .map(|(a, b)| a ^ b)
            .collect();
        let db: Vec<u8> = masked_db
            .iter()
            .zip(mgf1(&seed, masked_db.len()))
            .map(|(a, b)| a ^ b)
            .collect();
        if db[..H] != Sha256::digest(b"")[..] {
            bail!("OAEP 的 label 哈希不对");
        }
        let rest = &db[H..];
        let Some(position) = rest.iter().position(|byte| *byte != 0) else {
            bail!("OAEP 没有分隔符");
        };
        if rest[position] != 1 {
            bail!("OAEP 分隔符不对");
        }
        Ok(rest[position + 1..].to_vec())
    }

    /// PKCS#1 v1.5 加密填充：`00 02 <至少 8 个非零字节> 00 <明文>`。
    pub fn pkcs1_decode(message: &[u8]) -> anyhow::Result<Vec<u8>> {
        if message.len() < 11 || message[0] != 0 || message[1] != 2 {
            bail!("不是 PKCS#1 v1.5 编码");
        }
        let Some(position) = message[2..]
            .iter()
            .position(|byte| *byte == 0)
            .map(|position| position + 2)
        else {
            bail!("PKCS#1 v1.5 没有分隔符");
        };
        if position < 10 {
            bail!("PKCS#1 v1.5 填充太短");
        }
        Ok(message[position + 1..].to_vec())
    }
}

/* —————————————— 账号与组织 —————————————— */

fn user_agent() -> String {
    format!(
        "Zed/{VERSION} ({}; {})",
        std::env::consts::OS,
        std::env::consts::ARCH
    )
}

fn paid(plan: &str) -> bool {
    !plan.is_empty() && plan != "zed_free"
}

fn plan_label(plan: &str) -> &'static str {
    match plan {
        "zed_pro" => "Pro",
        "zed_pro_trial" => "Pro Trial",
        "zed_business" => "Business",
        "zed_student" => "Student",
        "zed_vip" => "VIP",
        _ => "Free",
    }
}

/// 选组织：有能调模型的套餐、且没关掉托管模型的；依次看已保存的、默认的、个人的、其它的。
/// 都没有时：已保存的 → 默认的 → 第一个。返回 (组织 id, 该组织的套餐)。
pub(crate) fn pick_org(me: &Value, saved: &str) -> (String, String) {
    let organizations: Vec<&Value> = me
        .get("organizations")
        .and_then(Value::as_array)
        .map(|list| list.iter().collect())
        .unwrap_or_default();
    if organizations.is_empty() {
        let plan = me
            .pointer("/plan/plan_v3")
            .and_then(Value::as_str)
            .unwrap_or_default();
        return (saved.to_string(), plan.to_string());
    }
    let plan_of = |id: &str| {
        me.pointer(&format!("/plans_by_organization/{id}"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    };
    let usable = |id: &str| {
        paid(&plan_of(id))
            && me
                .pointer(&format!(
                    "/configuration_by_organization/{id}/is_zed_model_provider_enabled"
                ))
                .and_then(Value::as_bool)
                != Some(false)
    };
    let id_of = |org: &&Value| {
        org.get("id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    };
    let exists = |id: &str| !id.is_empty() && organizations.iter().any(|org| id_of(org) == id);
    let default = me
        .get("default_organization_id")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let picked = [saved, default]
        .into_iter()
        .find(|id| exists(id) && usable(id))
        .map(str::to_string)
        .or_else(|| {
            organizations
                .iter()
                .find(|org| {
                    org.get("is_personal").and_then(Value::as_bool) == Some(true)
                        && usable(&id_of(org))
                })
                .map(id_of)
        })
        .or_else(|| organizations.iter().map(id_of).find(|id| usable(id)))
        .or_else(|| {
            [saved, default]
                .into_iter()
                .find(|id| exists(id))
                .map(str::to_string)
        })
        .unwrap_or_else(|| id_of(&organizations[0]));
    let plan = plan_of(&picked);
    (picked, plan)
}

fn account_name(me: &Value) -> String {
    let user = me.get("user").cloned().unwrap_or(Value::Null);
    ["github_login", "username", "name"]
        .iter()
        .find_map(|key| text_of(&user, key))
        .map(str::to_string)
        .or_else(|| user.get("legacy_user_id").map(|id| id.to_string()))
        .unwrap_or_else(|| "Zed account".to_string())
}

fn account_auth(credentials: &Value) -> anyhow::Result<String> {
    match (
        text_of(credentials, "userId"),
        text_of(credentials, "accessToken"),
    ) {
        (Some(user), Some(token)) => Ok(format!("{user} {token}")),
        _ => Err(fail(401, "Zed 凭据不完整，请重新登录")),
    }
}

fn with_system_id(
    request: reqwest::RequestBuilder,
    credentials: &Value,
) -> reqwest::RequestBuilder {
    match text_of(credentials, "systemId") {
        Some(id) => request.header("x-zed-system-id", id),
        None => request,
    }
}

async fn users_me(http: &reqwest::Client, credentials: &Value) -> anyhow::Result<Value> {
    let response = with_system_id(http.get(format!("{CLOUD}/client/users/me")), credentials)
        .header("authorization", account_auth(credentials)?)
        .header("content-type", "application/json")
        .header("user-agent", user_agent())
        .timeout(Duration::from_secs(20))
        .send()
        .await
        .map_err(|error| fail(502, format!("无法连接 Zed：{error}")))?;
    let status = response.status().as_u16();
    let text = response.text().await.unwrap_or_default();
    if status == 401 {
        return Err(fail(401, "Zed 不认这个登录了，请重新登录"));
    }
    if !(200..300).contains(&status) {
        return Err(fail(
            502,
            format!(
                "Zed /client/users/me 返回 {status}：{}",
                error_message(&text, status)
            ),
        ));
    }
    serde_json::from_str(&text).context("Zed 的账号信息不是 JSON")
}

/// 按组织换模型 token。返回 Err 的状态码：401 账号失效，402 这个组织没有套餐。
async fn mint_token(http: &reqwest::Client, credentials: &Value) -> anyhow::Result<String> {
    let organization = text_of(credentials, "organizationId").unwrap_or_default();
    let body = if organization.is_empty() {
        json!({})
    } else {
        json!({ "organization_id": organization })
    };
    let response = with_system_id(http.post(format!("{CLOUD}/client/llm_tokens")), credentials)
        .header("authorization", account_auth(credentials)?)
        .header("content-type", "application/json")
        .header("user-agent", user_agent())
        .json(&body)
        .timeout(Duration::from_secs(20))
        .send()
        .await
        .map_err(|error| fail(502, format!("无法连接 Zed：{error}")))?;
    let status = response.status().as_u16();
    let text = response.text().await.unwrap_or_default();
    match status {
        200..=299 => {}
        401 => return Err(fail(401, "Zed 不认这个登录了，请重新登录")),
        402 => {
            return Err(fail(
                402,
                format!(
                    "这个 Zed 组织没有可用的套餐：{}",
                    error_message(&text, status)
                ),
            ));
        }
        _ => {
            return Err(fail(
                502,
                format!(
                    "Zed 换取模型 token 失败：{} ({status})",
                    error_message(&text, status)
                ),
            ));
        }
    }
    let value: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
    text_of(&value, "token")
        .map(str::to_string)
        .ok_or_else(|| fail(502, "Zed 没有返回模型 token"))
}

/// 换一个新的模型 token 写进凭据；组织没有套餐（402）时重新选一次组织再试。
async fn renew(http: &reqwest::Client, credentials: &Value) -> anyhow::Result<Value> {
    let mut updated = credentials.clone();
    let token = match mint_token(http, &updated).await {
        Ok(token) => token,
        Err(error) if status_of(&error) == 402 => {
            let me = users_me(http, &updated).await?;
            let (organization, plan) =
                pick_org(&me, text_of(&updated, "organizationId").unwrap_or_default());
            if Some(organization.as_str()) == text_of(&updated, "organizationId") {
                return Err(error);
            }
            updated["organizationId"] = json!(organization);
            updated["plan"] = json!(plan);
            mint_token(http, &updated).await?
        }
        Err(error) => return Err(error),
    };
    updated["llmToken"] = json!(token);
    updated["llmTokenExpiresAt"] = json!(now_ms() + LLM_TOKEN_TTL_MS);
    Ok(updated)
}

fn status_of(error: &anyhow::Error) -> u16 {
    error
        .downcast_ref::<crate::gateway::subscriptions::SubscriptionError>()
        .map(|error| error.status)
        .unwrap_or(0)
}

/// 账号凭据（user_id + access token）→ 完整的登录：读账号、选组织、换模型 token。
async fn finish_account(
    http: &reqwest::Client,
    mut credentials: Value,
) -> anyhow::Result<LoginDone> {
    let me = users_me(http, &credentials).await?;
    let (organization, plan) = pick_org(
        &me,
        text_of(&credentials, "organizationId").unwrap_or_default(),
    );
    if me
        .pointer(&format!(
            "/configuration_by_organization/{organization}/is_zed_model_provider_enabled"
        ))
        .and_then(Value::as_bool)
        == Some(false)
    {
        bail!("这个 Zed 组织关掉了 Zed 托管的模型");
    }
    let account = account_name(&me);
    credentials["organizationId"] = json!(organization);
    credentials["plan"] = json!(plan);
    credentials["account"] = json!(account);
    let credentials = match renew(http, &credentials).await {
        Ok(updated) => updated,
        Err(error) if status_of(&error) == 402 => bail!(
            "这个 Zed 账号没有包含托管模型的套餐（当前：{}）",
            plan_label(&plan)
        ),
        Err(error) => return Err(error),
    };
    Ok(LoginDone {
        credentials,
        account,
    })
}

/* —————————————— 登录 —————————————— */

struct BrowserLogin {
    listener: tokio::net::TcpListener,
    key: rsa::PrivateKey,
    system_id: String,
}

fn decode_b64url(text: &str) -> anyhow::Result<Vec<u8>> {
    let trimmed = text.trim().trim_end_matches('=');
    base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(trimmed)
        .or_else(|_| base64::engine::general_purpose::STANDARD_NO_PAD.decode(trimmed))
        .context("回调里的 token 不是 base64")
}

/// 回调参数 → (user_id, 明文 access token)。
fn parse_callback(
    params: &std::collections::BTreeMap<String, String>,
    key: &rsa::PrivateKey,
) -> anyhow::Result<Option<(String, String)>> {
    let (Some(user), Some(token)) = (params.get("user_id"), params.get("access_token")) else {
        return Ok(None);
    };
    let plain = key.decrypt(&decode_b64url(token)?)?;
    let token = String::from_utf8(plain).context("解出的 token 不是文本")?;
    Ok(Some((user.trim().to_string(), token)))
}

async fn wait_browser(login: BrowserLogin) -> anyhow::Result<Value> {
    let BrowserLogin {
        listener,
        key,
        system_id,
    } = login;
    tokio::time::timeout(LOGIN_TIMEOUT, async {
        loop {
            let (mut stream, request) = editor_kit::next_request(&listener).await?;
            match parse_callback(&request.query, &key) {
                Ok(Some((user, token))) => {
                    editor_kit::respond(&mut stream, "302 Found", &[("location", SUCCEEDED)], "", b"").await;
                    return anyhow::Ok(json!({ "userId": user, "accessToken": token, "systemId": system_id, "source": "browser" }));
                }
                Ok(None) => editor_kit::respond(&mut stream, "404 Not Found", &[], "", b"").await,
                Err(error) => {
                    editor_kit::respond(
                        &mut stream,
                        "200 OK",
                        &[],
                        "text/html; charset=utf-8",
                        &editor_kit::page("登录没有完成，请回到 Codex++ 重新登录。"),
                    )
                    .await;
                    return Err(error);
                }
            }
        }
    })
    .await
    .map_err(|_| anyhow::anyhow!("登录超时：5 分钟内没有完成 Zed 登录"))?
}

/// macOS 钥匙串里 Zed 保存的登录（Internet 密码，服务器 https://zed.dev，账号是 user_id）。
fn read_keychain() -> anyhow::Result<Value> {
    if !cfg!(target_os = "macos") {
        bail!("导入本机 Zed 登录目前只支持 macOS；其它系统请用浏览器登录");
    }
    let attributes =
        editor_kit::command_output("/usr/bin/security", &["find-internet-password", "-s", SITE])
            .ok_or_else(|| anyhow::anyhow!("钥匙串里没有 Zed 的登录，请先在 Zed 里登录"))?;
    let user = keychain_account(&attributes)
        .ok_or_else(|| anyhow::anyhow!("钥匙串里的 Zed 登录没有账号"))?;
    let token = editor_kit::command_output(
        "/usr/bin/security",
        &["find-internet-password", "-s", SITE, "-w"],
    )
    .ok_or_else(|| anyhow::anyhow!("没能读出钥匙串里的 Zed 登录（可能拒绝了授权）"))?;
    Ok(json!({ "userId": user, "accessToken": token, "source": "keychain" }))
}

/// `security find-internet-password` 的输出里的账号：`"acct"<blob>="12345"`。
fn keychain_account(attributes: &str) -> Option<String> {
    attributes.lines().find_map(|line| {
        let rest = line.trim().strip_prefix("\"acct\"<blob>=")?;
        let value = rest.trim().trim_matches('"').to_string();
        (!value.is_empty() && value != "<NULL>").then_some(value)
    })
}

/* —————————————— 模型 —————————————— */

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Provider {
    Anthropic,
    OpenAi,
    Google,
    XAi,
}

impl Provider {
    fn parse(text: &str) -> Option<Self> {
        match text {
            "anthropic" => Some(Self::Anthropic),
            "open_ai" => Some(Self::OpenAi),
            "google" => Some(Self::Google),
            "x_ai" => Some(Self::XAi),
            _ => None,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Anthropic => "anthropic",
            Self::OpenAi => "open_ai",
            Self::Google => "google",
            Self::XAi => "x_ai",
        }
    }

    /// 列表里没有时按名字猜。
    fn guess(model: &str) -> Self {
        let lower = model.to_ascii_lowercase();
        if lower.starts_with("claude") {
            Self::Anthropic
        } else if lower.starts_with("gemini") {
            Self::Google
        } else if lower.starts_with("grok") {
            Self::XAi
        } else {
            Self::OpenAi
        }
    }
}

fn providers() -> &'static Mutex<HashMap<String, Provider>> {
    static PROVIDERS: OnceLock<Mutex<HashMap<String, Provider>>> = OnceLock::new();
    PROVIDERS.get_or_init(Default::default)
}

fn provider_for(model: &str) -> Provider {
    providers()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .get(model)
        .copied()
        .unwrap_or_else(|| Provider::guess(model))
}

/// `/models` → (id, 厂商)，去掉停用的和不认识的厂商；默认模型排第一。
fn parse_models(value: &Value) -> Vec<(String, Provider)> {
    let default = value
        .get("default_model")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let mut models: Vec<(String, Provider)> = value
        .get("models")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|model| model.get("is_disabled").and_then(Value::as_bool) != Some(true))
        .filter_map(|model| {
            let id = text_of(model, "id")?;
            let provider = Provider::parse(model.get("provider").and_then(Value::as_str)?)?;
            Some((id.to_string(), provider))
        })
        .collect();
    if let Some(position) = models.iter().position(|(id, _)| id == default) {
        let item = models.remove(position);
        models.insert(0, item);
    }
    models
}

const DEFAULT_MODELS: &[&str] = &[
    "claude-sonnet-5",
    "claude-opus-5-5",
    "claude-haiku-4-5",
    "gpt-5.5",
    "gemini-3.5-flash",
    "grok-4.7",
];

/* —————————————— 请求：各厂商的格式 —————————————— */

/// xAI：Chat Completions，`max_tokens` 改成 `max_completion_tokens`，角色只认 system/user/assistant/tool。
fn xai_request(request: &Value) -> Value {
    let mut body = serde_json::Map::new();
    for key in [
        "model",
        "messages",
        "temperature",
        "top_p",
        "stop",
        "tools",
        "tool_choice",
        "parallel_tool_calls",
        "reasoning_effort",
        "max_completion_tokens",
    ] {
        if let Some(value) = request.get(key) {
            body.insert(key.to_string(), value.clone());
        }
    }
    if let Some(max) = request
        .get("max_tokens")
        .filter(|_| !body.contains_key("max_completion_tokens"))
    {
        body.insert("max_completion_tokens".to_string(), max.clone());
    }
    if let Some(messages) = body.get_mut("messages").and_then(Value::as_array_mut) {
        for message in messages {
            if message.get("role").and_then(Value::as_str) == Some("developer") {
                message["role"] = json!("system");
            }
            if let Some(object) = message.as_object_mut() {
                object.remove("reasoning_content");
            }
        }
    }
    body.insert("stream".to_string(), json!(true));
    body.insert(
        "stream_options".to_string(),
        json!({ "include_usage": true }),
    );
    Value::Object(body)
}

/// Chat 的内容片段 → Responses 的内容（`input_text` / `output_text` / `input_image`）。
fn responses_content(message: &Value, assistant: bool) -> Vec<Value> {
    let kind = if assistant {
        "output_text"
    } else {
        "input_text"
    };
    match message.get("content") {
        Some(Value::String(text)) if !text.is_empty() => vec![json!({ "type": kind, "text": text })],
        Some(Value::Array(parts)) => parts
            .iter()
            .filter_map(|part| match part.get("type").and_then(Value::as_str) {
                Some("text") => Some(json!({ "type": kind, "text": part.get("text").and_then(Value::as_str).unwrap_or_default() })),
                Some("image_url") if !assistant => {
                    let url = part.pointer("/image_url/url").or_else(|| part.get("image_url")).and_then(Value::as_str)?;
                    Some(json!({ "type": "input_image", "image_url": url }))
                }
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    }
}

/// OpenAI：Chat 请求 → Responses 请求。
fn responses_request(request: &Value) -> Value {
    let mut input = Vec::new();
    for message in request
        .get("messages")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        match message
            .get("role")
            .and_then(Value::as_str)
            .unwrap_or("user")
        {
            "system" | "developer" => {
                let content = responses_content(message, false);
                if !content.is_empty() {
                    input.push(json!({ "type": "message", "role": "system", "content": content }));
                }
            }
            "assistant" => {
                let content = responses_content(message, true);
                if !content.is_empty() {
                    input.push(
                        json!({ "type": "message", "role": "assistant", "content": content }),
                    );
                }
                for call in message
                    .get("tool_calls")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                {
                    let arguments = match call.pointer("/function/arguments") {
                        Some(Value::String(text)) => text.clone(),
                        Some(other) => other.to_string(),
                        None => "{}".to_string(),
                    };
                    input.push(json!({
                        "type": "function_call",
                        "call_id": call.get("id").and_then(Value::as_str).unwrap_or_default(),
                        "name": call.pointer("/function/name").and_then(Value::as_str).unwrap_or_default(),
                        "arguments": arguments,
                    }));
                }
            }
            "tool" => input.push(json!({
                "type": "function_call_output",
                "call_id": message.get("tool_call_id").and_then(Value::as_str).unwrap_or_default(),
                "output": content_text(message),
            })),
            _ => {
                let content = responses_content(message, false);
                if !content.is_empty() {
                    input.push(json!({ "type": "message", "role": "user", "content": content }));
                }
            }
        }
    }
    let tools: Vec<Value> = request
        .get("tools")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|tool| {
            let function = tool.get("function")?;
            Some(json!({
                "type": "function",
                "name": function.get("name")?.as_str()?,
                "description": function.get("description").and_then(Value::as_str).unwrap_or_default(),
                "parameters": function.get("parameters").cloned().unwrap_or_else(|| json!({ "type": "object", "properties": {} })),
                "strict": false,
            }))
        })
        .collect();
    let mut body = json!({
        "model": request.get("model").cloned().unwrap_or(Value::Null),
        "input": input,
        "stream": true,
    });
    if !tools.is_empty() {
        body["tools"] = json!(tools);
        body["tool_choice"] = match request.get("tool_choice") {
            Some(Value::String(choice))
                if matches!(choice.as_str(), "auto" | "none" | "required") =>
            {
                json!(choice)
            }
            Some(Value::Object(_)) => json!("required"),
            _ => json!("auto"),
        };
    }
    if let Some(max) = request
        .get("max_completion_tokens")
        .or_else(|| request.get("max_tokens"))
        .and_then(Value::as_u64)
    {
        body["max_output_tokens"] = json!(max);
    }
    if let Some(effort) = reasoning_effort(request).filter(|effort| effort != "off") {
        let effort = match effort.as_str() {
            "max" | "ultra" => "xhigh".to_string(),
            other => other.to_string(),
        };
        body["reasoning"] = json!({ "effort": effort, "summary": "auto" });
    }
    body
}

/// JSON Schema → Gemini 认的子集。
fn gemini_schema(schema: &Value) -> Value {
    let Some(object) = schema.as_object() else {
        return json!({ "type": "object" });
    };
    let mut out = serde_json::Map::new();
    match object.get("type") {
        Some(Value::Array(types)) => {
            let kinds: Vec<&str> = types.iter().filter_map(Value::as_str).collect();
            if let Some(kind) = kinds.iter().find(|kind| **kind != "null") {
                out.insert("type".to_string(), json!(kind));
            }
            if kinds.contains(&"null") {
                out.insert("nullable".to_string(), json!(true));
            }
        }
        Some(kind @ Value::String(_)) => {
            out.insert("type".to_string(), kind.clone());
        }
        _ => {}
    }
    for key in [
        "description",
        "nullable",
        "required",
        "minItems",
        "maxItems",
        "minimum",
        "maximum",
    ] {
        if let Some(value) = object.get(key) {
            out.insert(key.to_string(), value.clone());
        }
    }
    if let Some(values) = object.get("enum").and_then(Value::as_array) {
        out.insert(
            "enum".to_string(),
            json!(
                values
                    .iter()
                    .map(|value| match value {
                        Value::String(text) => json!(text),
                        other => json!(other.to_string()),
                    })
                    .collect::<Vec<_>>()
            ),
        );
        out.insert("type".to_string(), json!("string"));
    }
    if let Some(properties) = object.get("properties").and_then(Value::as_object) {
        let mapped: serde_json::Map<String, Value> = properties
            .iter()
            .map(|(key, value)| (key.clone(), gemini_schema(value)))
            .collect();
        out.insert("properties".to_string(), Value::Object(mapped));
        out.entry("type".to_string()).or_insert(json!("object"));
    }
    if let Some(items) = object.get("items") {
        out.insert("items".to_string(), gemini_schema(items));
    }
    if let Some(any_of) = object
        .get("anyOf")
        .or_else(|| object.get("oneOf"))
        .and_then(Value::as_array)
    {
        out.insert(
            "anyOf".to_string(),
            json!(any_of.iter().map(gemini_schema).collect::<Vec<_>>()),
        );
    }
    if out.is_empty() {
        out.insert("type".to_string(), json!("object"));
    }
    Value::Object(out)
}

/// Google：Chat 请求 → generateContent 请求（`model: models/<id>`）。
fn gemini_request(request: &Value, model: &str) -> Value {
    let messages = request
        .get("messages")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let names = editor_kit::tool_names(&messages);
    let mut system = Vec::new();
    let mut contents: Vec<Value> = Vec::new();
    let mut push = |role: &str, parts: Vec<Value>| {
        if parts.is_empty() {
            return;
        }
        if let Some(last) = contents.last_mut()
            && last.get("role").and_then(Value::as_str) == Some(role)
            && let Some(existing) = last.get_mut("parts").and_then(Value::as_array_mut)
        {
            existing.extend(parts);
            return;
        }
        contents.push(json!({ "role": role, "parts": parts }));
    };
    for message in &messages {
        match message
            .get("role")
            .and_then(Value::as_str)
            .unwrap_or("user")
        {
            "system" | "developer" => {
                let text = content_text(message);
                if !text.is_empty() {
                    system.push(json!({ "text": text }));
                }
            }
            "assistant" => {
                let mut parts = Vec::new();
                let text = content_text(message);
                if !text.is_empty() {
                    parts.push(json!({ "text": text }));
                }
                for call in message
                    .get("tool_calls")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                {
                    parts.push(json!({
                        "functionCall": {
                            "name": call.pointer("/function/name").and_then(Value::as_str).unwrap_or_default(),
                            "args": arguments_object(call.pointer("/function/arguments")),
                        },
                        // 回放的调用没有原签名：用官方给的占位值跳过校验。
                        "thoughtSignature": "skip_thought_signature_validator",
                    }));
                }
                push("model", parts);
            }
            "tool" => {
                let id = message
                    .get("tool_call_id")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                push(
                    "user",
                    vec![json!({ "functionResponse": {
                        "name": names.get(id).map(String::as_str).unwrap_or("tool"),
                        "response": { "content": content_text(message) },
                    } })],
                );
            }
            _ => {
                let mut parts = Vec::new();
                match message.get("content") {
                    Some(Value::String(text)) if !text.is_empty() => {
                        parts.push(json!({ "text": text }))
                    }
                    Some(Value::Array(items)) => {
                        for item in items {
                            match item.get("type").and_then(Value::as_str) {
                                Some("text") => parts.push(json!({ "text": item.get("text").and_then(Value::as_str).unwrap_or_default() })),
                                Some("image_url") => {
                                    let url = item.pointer("/image_url/url").or_else(|| item.get("image_url")).and_then(Value::as_str).unwrap_or_default();
                                    if let Some((mime, data)) = url.strip_prefix("data:").and_then(|rest| rest.split_once(";base64,")) {
                                        parts.push(json!({ "inlineData": { "mimeType": mime, "data": data } }));
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                    _ => {}
                }
                push("user", parts);
            }
        }
    }
    let mut body = json!({ "model": format!("models/{model}"), "contents": contents });
    if !system.is_empty() {
        body["systemInstruction"] = json!({ "parts": system });
    }
    let declarations: Vec<Value> = request
        .get("tools")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|tool| {
            let function = tool.get("function")?;
            let mut declaration = json!({
                "name": function.get("name")?.as_str()?,
                "description": function.get("description").and_then(Value::as_str).unwrap_or_default(),
            });
            if let Some(parameters) = function.get("parameters").filter(|schema| {
                schema.get("properties").and_then(Value::as_object).is_some_and(|properties| !properties.is_empty())
            }) {
                declaration["parameters"] = gemini_schema(parameters);
            }
            Some(declaration)
        })
        .collect();
    if !declarations.is_empty() {
        body["tools"] = json!([{ "functionDeclarations": declarations }]);
        let mode = match request.get("tool_choice") {
            Some(Value::String(choice)) if choice == "none" => Some("NONE"),
            Some(Value::String(choice)) if choice == "required" => Some("ANY"),
            Some(Value::Object(_)) => Some("ANY"),
            _ => None,
        };
        if let Some(mode) = mode {
            body["toolConfig"] = json!({ "functionCallingConfig": { "mode": mode } });
        }
    }
    let mut config = serde_json::Map::new();
    if let Some(max) = request
        .get("max_completion_tokens")
        .or_else(|| request.get("max_tokens"))
        .and_then(Value::as_u64)
    {
        config.insert("maxOutputTokens".to_string(), json!(max));
    }
    if let Some(temperature) = request.get("temperature").and_then(Value::as_f64) {
        config.insert("temperature".to_string(), json!(temperature));
    }
    if reasoning_effort(request).is_none_or(|effort| !matches!(effort.as_str(), "none" | "off")) {
        config.insert(
            "thinkingConfig".to_string(),
            json!({ "includeThoughts": true }),
        );
    }
    if !config.is_empty() {
        body["generationConfig"] = Value::Object(config);
    }
    body
}

/// 发给 `/completions` 的请求体。
fn completion_body(request: &Value, model: &str, provider: Provider) -> anyhow::Result<Value> {
    let provider_request = match provider {
        Provider::Anthropic => editor_kit::anthropic_request(request, false)?,
        Provider::OpenAi => responses_request(request),
        Provider::XAi => xai_request(request),
        Provider::Google => gemini_request(request, model),
    };
    Ok(json!({ "provider": provider.name(), "model": model, "provider_request": provider_request }))
}

/* —————————————— 答复：各厂商的事件 → Chat SSE —————————————— */

/// OpenAI Responses 的流事件 → Chat SSE。
#[derive(Debug, Default)]
struct ResponsesEvents {
    /// 输出项 id → 调用 id。
    calls: HashMap<String, String>,
    /// 已经流过参数的调用。
    streamed: HashSet<String>,
}

impl ResponsesEvents {
    fn start_call(&mut self, sse: &mut ChatSse, item: &Value) -> Vec<u8> {
        let item_id = text_of(item, "id").unwrap_or_default().to_string();
        let call_id = text_of(item, "call_id")
            .map(str::to_string)
            .unwrap_or_else(|| item_id.clone());
        let call_id = if call_id.is_empty() {
            editor_kit::new_call_id()
        } else {
            call_id
        };
        self.calls.insert(item_id, call_id.clone());
        let arguments = item
            .get("arguments")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if !arguments.is_empty() {
            self.streamed.insert(call_id.clone());
        }
        sse.tool_call(
            &call_id,
            text_of(item, "name").unwrap_or_default(),
            arguments,
        )
    }

    fn push(&mut self, sse: &mut ChatSse, event: &Value) -> anyhow::Result<Vec<u8>> {
        let delta = || {
            event
                .get("delta")
                .and_then(Value::as_str)
                .unwrap_or_default()
        };
        match event
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default()
        {
            "response.output_text.delta" => Ok(sse.text(delta())),
            "response.reasoning_summary_text.delta" | "response.reasoning_text.delta" => {
                Ok(sse.reasoning(delta()))
            }
            "response.output_item.added" => {
                let item = event.get("item").cloned().unwrap_or(Value::Null);
                if item.get("type").and_then(Value::as_str) == Some("function_call") {
                    return Ok(self.start_call(sse, &item));
                }
                Ok(Vec::new())
            }
            "response.function_call_arguments.delta" => {
                let item_id = text_of(event, "item_id").unwrap_or_default();
                match self.calls.get(item_id).cloned() {
                    Some(call_id) => {
                        self.streamed.insert(call_id.clone());
                        Ok(sse.tool_arguments(&call_id, delta()))
                    }
                    None => Ok(Vec::new()),
                }
            }
            "response.output_item.done" => {
                let item = event.get("item").cloned().unwrap_or(Value::Null);
                if item.get("type").and_then(Value::as_str) != Some("function_call") {
                    return Ok(Vec::new());
                }
                let item_id = text_of(&item, "id").unwrap_or_default();
                let Some(call_id) = self.calls.get(item_id).cloned() else {
                    return Ok(self.start_call(sse, &item));
                };
                if self.streamed.contains(&call_id) {
                    return Ok(Vec::new());
                }
                self.streamed.insert(call_id.clone());
                Ok(sse.tool_arguments(
                    &call_id,
                    item.get("arguments")
                        .and_then(Value::as_str)
                        .unwrap_or_default(),
                ))
            }
            "response.completed" | "response.incomplete" => {
                let response = event.get("response").cloned().unwrap_or(Value::Null);
                let usage = response
                    .get("usage")
                    .filter(|usage| usage.is_object())
                    .map(|usage| {
                        let get = |pointer: &str| {
                            usage.pointer(pointer).and_then(Value::as_u64).unwrap_or(0)
                        };
                        Usage {
                            prompt_tokens: get("/input_tokens"),
                            completion_tokens: get("/output_tokens"),
                            cached_tokens: get("/input_tokens_details/cached_tokens"),
                            reasoning_tokens: get("/output_tokens_details/reasoning_tokens"),
                        }
                    });
                let reason = (response.get("status").and_then(Value::as_str) == Some("incomplete")
                    || event.get("type").and_then(Value::as_str) == Some("response.incomplete"))
                .then_some("length")
                .filter(|_| !sse.has_tool_calls());
                Ok(sse.finish(reason, usage))
            }
            "response.failed" | "error" => {
                let error = event
                    .pointer("/response/error")
                    .or_else(|| event.get("error"))
                    .unwrap_or(event);
                let message = error
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("OpenAI 返回错误");
                let code = error
                    .get("code")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                Err(fail(code_status(code, None), message.to_string()))
            }
            _ => Ok(Vec::new()),
        }
    }
}

/// Gemini generateContent 的流片段 → Chat SSE。
#[derive(Debug, Default)]
struct GeminiEvents {
    usage: Option<Usage>,
    reason: Option<&'static str>,
}

impl GeminiEvents {
    fn push(&mut self, sse: &mut ChatSse, event: &Value) -> anyhow::Result<Vec<u8>> {
        if let Some(error) = event.get("error") {
            let message = error
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("Gemini 返回错误");
            let status = error
                .get("code")
                .and_then(Value::as_u64)
                .filter(|code| (400..600).contains(code))
                .unwrap_or(502);
            return Err(fail(status as u16, message.to_string()));
        }
        let mut out = Vec::new();
        if let Some(candidate) = event
            .get("candidates")
            .and_then(|candidates| candidates.get(0))
        {
            for part in candidate
                .pointer("/content/parts")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                if let Some(call) = part.get("functionCall") {
                    let id = text_of(call, "id")
                        .map(str::to_string)
                        .unwrap_or_else(editor_kit::new_call_id);
                    let arguments = call
                        .get("args")
                        .cloned()
                        .unwrap_or_else(|| json!({}))
                        .to_string();
                    out.extend(sse.tool_call(
                        &id,
                        text_of(call, "name").unwrap_or_default(),
                        &arguments,
                    ));
                } else if let Some(text) = part.get("text").and_then(Value::as_str) {
                    if part.get("thought").and_then(Value::as_bool) == Some(true) {
                        out.extend(sse.reasoning(text));
                    } else {
                        out.extend(sse.text(text));
                    }
                }
            }
            if let Some(reason) = candidate.get("finishReason").and_then(Value::as_str) {
                self.reason = match reason {
                    "MAX_TOKENS" => Some("length"),
                    "SAFETY" | "RECITATION" | "BLOCKLIST" | "PROHIBITED_CONTENT" | "SPII" => {
                        Some("content_filter")
                    }
                    _ => None,
                };
            }
        }
        if let Some(usage) = event.get("usageMetadata") {
            let get = |key: &str| usage.get(key).and_then(Value::as_u64).unwrap_or(0);
            let thoughts = get("thoughtsTokenCount");
            self.usage = Some(Usage {
                prompt_tokens: get("promptTokenCount"),
                completion_tokens: get("candidatesTokenCount") + thoughts,
                cached_tokens: get("cachedContentTokenCount"),
                reasoning_tokens: thoughts,
            });
        }
        Ok(out)
    }

    fn finish(&mut self, sse: &mut ChatSse) -> Vec<u8> {
        sse.finish(self.reason, self.usage)
    }
}

/// 按厂商分派的转换器。
enum Mapper {
    Anthropic(AnthropicEvents),
    Chat(ChatChunks),
    Responses(ResponsesEvents),
    Gemini(GeminiEvents),
}

impl Mapper {
    fn new(provider: Provider) -> Self {
        match provider {
            Provider::Anthropic => Self::Anthropic(AnthropicEvents::default()),
            Provider::XAi => Self::Chat(ChatChunks::default()),
            Provider::OpenAi => Self::Responses(ResponsesEvents::default()),
            Provider::Google => Self::Gemini(GeminiEvents::default()),
        }
    }

    fn push(&mut self, sse: &mut ChatSse, event: &Value) -> anyhow::Result<Vec<u8>> {
        match self {
            Self::Anthropic(state) => state.push(sse, event),
            Self::Chat(state) => state.push(sse, event),
            Self::Responses(state) => state.push(sse, event),
            Self::Gemini(state) => state.push(sse, event),
        }
    }

    /// 正常结束（`stream_ended`）时收尾。
    fn finish(&mut self, sse: &mut ChatSse) -> Vec<u8> {
        match self {
            Self::Anthropic(state) => state.finish(sse),
            Self::Chat(state) => state.finish(sse),
            Self::Responses(_) => sse.finish(None, None),
            Self::Gemini(state) => state.finish(sse),
        }
    }
}

/// Zed 的错误码 → 状态码。
fn code_status(code: &str, upstream: Option<u64>) -> u16 {
    if let Some(status) = upstream.filter(|status| (400..600).contains(status)) {
        return status as u16;
    }
    let lower = code.to_ascii_lowercase();
    if let Some(number) = lower
        .strip_prefix("upstream_http_")
        .or_else(|| lower.strip_prefix("http_"))
        .and_then(|rest| rest.parse::<u16>().ok())
    {
        return if (400..600).contains(&number) {
            number
        } else {
            502
        };
    }
    if lower.contains("rate_limit") {
        429
    } else if lower.contains("overloaded") {
        529
    } else if lower.contains("billing") || lower.contains("payment") {
        402
    } else if lower.contains("context_length") {
        400
    } else {
        502
    }
}

fn error_message(text: &str, status: u16) -> String {
    let value: Value = serde_json::from_str(text).unwrap_or(Value::Null);
    ["message", "error", "code"]
        .iter()
        .find_map(|key| text_of(&value, key).map(str::to_string))
        .or_else(|| {
            let trimmed = text.trim();
            (!trimmed.is_empty() && trimmed.len() < 500).then(|| trimmed.to_string())
        })
        .unwrap_or_else(|| format!("HTTP {status}"))
}

/// `/completions` 的非 2xx。
fn http_failure(status: u16, text: &str) -> anyhow::Error {
    let value: Value = serde_json::from_str(text).unwrap_or(Value::Null);
    let mapped = match status {
        401 => 401,
        402 => 402,
        _ => code_status(
            value
                .get("code")
                .and_then(Value::as_str)
                .unwrap_or_default(),
            value
                .get("upstream_status")
                .and_then(Value::as_u64)
                .or(Some(u64::from(status))),
        ),
    };
    let mut message = error_message(text, status);
    if let Some(after) = value.get("retry_after").and_then(Value::as_f64) {
        message.push_str(&format!("（{after} 秒后重试）"));
    }
    if mapped == 402 {
        message = format!("Zed 套餐不含托管模型或额度已用完：{message}");
    }
    fail(mapped, format!("Zed：{message}"))
}

/// 逐行答复的状态。
struct ZedStream {
    mapper: Mapper,
    /// 服务端支持状态行（有 `stream_ended`）。
    wrapped: bool,
    ended: bool,
}

impl ZedStream {
    fn push(&mut self, sse: &mut ChatSse, line: &str) -> anyhow::Result<Vec<u8>> {
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            return Ok(Vec::new());
        };
        if !self.wrapped {
            return self.mapper.push(sse, &value);
        }
        if let Some(event) = value.get("event") {
            return self.mapper.push(sse, event);
        }
        match value.get("status") {
            Some(Value::String(status)) if status == "stream_ended" => {
                self.ended = true;
                Ok(if sse.is_finished() {
                    Vec::new()
                } else {
                    self.mapper.finish(sse)
                })
            }
            Some(Value::Object(object)) if object.contains_key("failed") => {
                let failed = &object["failed"];
                let code = failed
                    .get("code")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let mut message = failed
                    .get("message")
                    .and_then(Value::as_str)
                    .filter(|message| !message.is_empty())
                    .unwrap_or(code)
                    .to_string();
                if let Some(after) = failed.get("retry_after").and_then(Value::as_f64) {
                    message.push_str(&format!("（{after} 秒后重试）"));
                }
                Err(fail(code_status(code, None), format!("Zed：{message}")))
            }
            _ => Ok(Vec::new()),
        }
    }
}

async fn send_completion(
    http: &reqwest::Client,
    token: &str,
    body: &Value,
) -> anyhow::Result<reqwest::Response> {
    http.post(format!("{CLOUD}/completions"))
        .bearer_auth(token)
        .header("content-type", "application/json")
        .header("x-zed-version", VERSION)
        .header("x-zed-client-supports-status-messages", "true")
        .header(
            "x-zed-client-supports-stream-ended-request-completion-status",
            "true",
        )
        .header("user-agent", user_agent())
        .json(body)
        .send()
        .await
        .map_err(|error| fail(502, format!("无法连接 Zed：{error}")))
}

/// 模型 token 过期（401 或过期头）。
fn token_stale(response: &reqwest::Response) -> bool {
    response.status().as_u16() == 401
        || ["x-zed-expired-token", "x-zed-outdated-token"]
            .iter()
            .any(|name| {
                response
                    .headers()
                    .get(*name)
                    .is_some_and(|value| !value.is_empty())
            })
}

async fn stream_reply(mut out: Emitter, response: reqwest::Response, mut state: ZedStream) {
    let mut bytes = response.bytes_stream();
    let mut lines = LineReader::default();
    loop {
        let chunk = match bytes.next().await {
            Some(Ok(chunk)) => chunk,
            Some(Err(error)) => {
                cut_short(&mut out, &mut state, Some(format!("{error}"))).await;
                return;
            }
            None => break,
        };
        for line in lines.push(&chunk) {
            if !handle_line(&mut out, &mut state, &line).await {
                return;
            }
        }
    }
    if let Some(line) = lines.finish()
        && !handle_line(&mut out, &mut state, &line).await
    {
        return;
    }
    if out.sse.is_finished() {
        return;
    }
    if state.ended || !state.wrapped {
        let bytes = state.mapper.finish(&mut out.sse);
        out.send(bytes).await;
        return;
    }
    cut_short(&mut out, &mut state, None).await;
}

/// 处理一行；返回 false 表示该停了。
async fn handle_line(out: &mut Emitter, state: &mut ZedStream, line: &str) -> bool {
    match state.push(&mut out.sse, line) {
        Ok(bytes) => out.send(bytes).await && !(state.ended && out.sse.is_finished()),
        Err(error) => {
            out.fail(error).await;
            false
        }
    }
}

/// 没等到 `stream_ended` 流就断了：说过话就补一句说明正常收尾，没说过话报 502。
async fn cut_short(out: &mut Emitter, state: &mut ZedStream, detail: Option<String>) {
    if out.sse.is_finished() {
        return;
    }
    if out.started() {
        let note = out
            .sse
            .text("\n\n(回复被截断：Zed 在说完之前关闭了连接，可以让它接着说。)");
        out.send(note).await;
        let bytes = state.mapper.finish(&mut out.sse);
        out.send(bytes).await;
    } else {
        let detail = detail
            .map(|detail| format!("：{detail}"))
            .unwrap_or_default();
        out.fail(fail(502, format!("Zed 的回复在说完之前就断了{detail}")))
            .await;
    }
}

/// 凭据里能用的模型 token；没有或快过期就现换一个（不写回）。
async fn llm_token(http: &reqwest::Client, credentials: &Value) -> anyhow::Result<String> {
    if let Some(token) = text_of(credentials, "llmToken")
        && !crate::gateway::subscriptions::oauth::expiring(credentials, "llmTokenExpiresAt", 60_000)
    {
        return Ok(token.to_string());
    }
    let renewed = renew(http, credentials).await?;
    Ok(text_of(&renewed, "llmToken")
        .unwrap_or_default()
        .to_string())
}

/* —————————————— Subscription —————————————— */

#[async_trait::async_trait]
impl Subscription for Zed {
    fn id(&self) -> &'static str {
        "zed"
    }

    fn name(&self) -> &'static str {
        "Zed"
    }

    fn login_methods(&self) -> &'static [LoginMethod] {
        &[
            LoginMethod::Browser,
            LoginMethod::ImportLocal,
            LoginMethod::Paste,
        ]
    }

    fn note(&self) -> &'static str {
        "需要 Zed Pro（含试用）、Student 或 Business 套餐：可在浏览器里用 Zed 账号登录，或先在 Zed 编辑器里登录后从钥匙串导入（macOS）。"
    }

    async fn begin_login(
        &self,
        _http: &reqwest::Client,
        method: LoginMethod,
        input: &str,
    ) -> anyhow::Result<LoginStart> {
        match method {
            LoginMethod::Browser => {
                let listener = editor_kit::listen_first(&[0]).await?;
                let port = listener.local_addr()?.port();
                let key = tokio::task::spawn_blocking(|| {
                    rsa::PrivateKey::generate(2048, &mut editor_kit::random_bytes)
                })
                .await?;
                let public_key = base64::engine::general_purpose::URL_SAFE.encode(key.public_der());
                let system_id = uuid::Uuid::new_v4().to_string();
                let url = format!(
                    "{SITE}/native_app_signin?{}",
                    query(&[
                        ("native_app_port", &port.to_string()),
                        ("native_app_public_key", &public_key),
                        ("system_id", &system_id),
                    ])
                );
                let session = editor_kit::park(
                    self.id(),
                    BrowserLogin {
                        listener,
                        key,
                        system_id,
                    },
                );
                Ok(LoginStart {
                    open_url: Some(url),
                    message: "请在浏览器里登录 Zed，登录完会自动继续。".to_string(),
                    state: json!({ "method": "browser", "session": session }),
                    ..LoginStart::default()
                })
            }
            LoginMethod::ImportLocal => {
                let credentials = read_keychain()?;
                Ok(LoginStart {
                    message: "已读取 Zed 保存在钥匙串里的登录。".to_string(),
                    state: json!({ "method": "import", "credentials": credentials }),
                    ..LoginStart::default()
                })
            }
            LoginMethod::Paste => {
                let mut parts = input.split_whitespace();
                let (Some(user), Some(token)) = (parts.next(), parts.next()) else {
                    bail!("请粘贴「user_id access_token」（中间一个空格）");
                };
                Ok(LoginStart {
                    message: "正在校验。".to_string(),
                    state: json!({ "method": "import", "credentials": { "userId": user, "accessToken": token, "source": "paste" } }),
                    ..LoginStart::default()
                })
            }
            LoginMethod::DeviceCode => bail!("Zed 不支持设备码登录"),
        }
    }

    async fn finish_login(
        &self,
        http: &reqwest::Client,
        start: &LoginStart,
    ) -> anyhow::Result<LoginDone> {
        let state = &start.state;
        let credentials = match text_of(state, "method").unwrap_or_default() {
            "browser" => {
                let session = text_of(state, "session").unwrap_or_default();
                let login = editor_kit::unpark::<BrowserLogin>(self.id(), session)
                    .ok_or_else(|| anyhow::anyhow!("这次登录已失效，请重新开始"))?;
                wait_browser(login).await?
            }
            "import" => state.get("credentials").cloned().unwrap_or(Value::Null),
            other => bail!("不认识的登录状态：{other}"),
        };
        finish_account(http, credentials).await
    }

    async fn refresh(
        &self,
        http: &reqwest::Client,
        credentials: &Value,
    ) -> anyhow::Result<Option<Value>> {
        if text_of(credentials, "llmToken").is_some()
            && !crate::gateway::subscriptions::oauth::expiring(
                credentials,
                "llmTokenExpiresAt",
                REFRESH_AHEAD_MS,
            )
        {
            return Ok(None);
        }
        renew(http, credentials).await.map(Some)
    }

    async fn models(
        &self,
        http: &reqwest::Client,
        credentials: &Value,
    ) -> anyhow::Result<Vec<String>> {
        let fetch = |token: String| {
            http.get(format!("{CLOUD}/models"))
                .bearer_auth(token)
                .header("x-zed-client-supports-x-ai", "true")
                .header("user-agent", user_agent())
                .timeout(Duration::from_secs(15))
                .send()
        };
        let mut token = llm_token(http, credentials).await?;
        let mut response = fetch(token.clone()).await;
        if response.as_ref().is_ok_and(token_stale) {
            token = text_of(&renew(http, credentials).await?, "llmToken")
                .unwrap_or_default()
                .to_string();
            response = fetch(token).await;
        }
        let listed = match response {
            Ok(response) if response.status().is_success() => {
                parse_models(&response.json::<Value>().await.unwrap_or(Value::Null))
            }
            _ => Vec::new(),
        };
        if listed.is_empty() {
            return Ok(DEFAULT_MODELS.iter().map(|id| id.to_string()).collect());
        }
        let mut map = providers()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        for (id, provider) in &listed {
            map.insert(id.clone(), *provider);
        }
        Ok(listed.into_iter().map(|(id, _)| id).collect())
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
        let provider = provider_for(&model);
        let body = completion_body(&request, &model, provider)?;
        let token = llm_token(http, credentials).await?;
        let mut response = send_completion(http, &token, &body).await?;
        if token_stale(&response) {
            // 模型 token 过期了：现换一个再试一次。
            let renewed = renew(http, credentials).await?;
            response = send_completion(
                http,
                text_of(&renewed, "llmToken").unwrap_or_default(),
                &body,
            )
            .await?;
        }
        let status = response.status().as_u16();
        if !(200..300).contains(&status) {
            let text = response.text().await.unwrap_or_default();
            return Err(http_failure(status, &text));
        }
        let wrapped = response
            .headers()
            .contains_key("x-zed-server-supports-status-messages");
        let state = ZedStream {
            mapper: Mapper::new(provider),
            wrapped,
            ended: false,
        };
        Ok(editor_kit::spawn_chat(&model, move |out| {
            stream_reply(out, response, state)
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::rsa::{Big, PrivateKey};
    use super::*;
    use crate::gateway::subscriptions::chat::aggregate;

    fn unhex(text: &str) -> Vec<u8> {
        (0..text.len())
            .step_by(2)
            .map(|at| u8::from_str_radix(&text[at..at + 2], 16).unwrap())
            .collect()
    }

    /// 只用于测试的密钥（参考资料里的测试向量）。
    fn test_key() -> PrivateKey {
        PrivateKey {
            n: Big::from_be(&unhex(
                "cfc0c1afe657192c795645f8d5a9204a5ea60970bde3df91a8003d48674d437848f8ce6d59bdea5ba5baae01bb0856aefbf55b48c91b8166d8d6080b906ae1abb36ca74099799235c639094d9c4ff540f95f63902141dd63f117002ddf6844ca2bde14af824a40fbc1fa3be2f3bdb9e79e3b20223072e91593ee9bc2781689aadd28488cab134c5d1f7e82477cefaed84cfc30e5d589e7540052d9334eb9f37ec85f175bd256b12c29062a6bf58d3e9e79cc8627302e9ce116ccc4749786ed00b1b6c1512214d1c8c6bc738c969cc1a537314d645a92a2578eaa4a3fb8218b848d55c61867ca73083de7ce7d2f3a7865a45b831e67487d2a12acb80f8fd37b3b",
            )),
            e: Big::from_u64(65_537),
            d: Big::from_be(&unhex(
                "266a6fc64d73049a2733af4ba4829a1076710064e3101c7aac14a3405cadba50a739aebe24ae8c03f760ce254ec6eb747eedaf3c05c7fac5204a6f8167ea83a142d3d0178f699e02288dca6d19fee50c67305ed23b7c18ea8e9da0b05ad3c68895d5028133b7f44d3960c2ab24440c78ca5707899ae8dcb5db3d6f7ae5c6d11cbc483caf9ccd1d863a7c27e6c29abb56048954bcfeea26a8152ddfae20b7c9d141af32607ac1a8e456753ee80b987c27a8f2cdd382f6bc6c6f18d5b388ee8106dad01c0723c8b406f75237cc9fb24f21984070f4a41eae1568c67addad4b96855d5ee848e027b866dc547aeb0def7b3aead0fc98d6f2a83541cf2f45bb554f59",
            )),
        }
    }

    #[test]
    fn bignum_basics() {
        let a = Big::from_be(&unhex("0123456789abcdef0011223344556677"));
        assert_eq!(a.to_be(0), unhex("0123456789abcdef0011223344556677"));
        assert_eq!(a.to_be(20)[..4], [0, 0, 0, 0]);
        let b = Big::from_u64(u64::MAX);
        assert_eq!(a.add(&b).sub(&b), a);
        let (quotient, rest) = a
            .mul(&Big::from_u64(65_537))
            .add(&Big::from_u64(5))
            .div_small(65_537);
        assert_eq!((quotient, rest), (a.clone(), 5));
        // 3^200 mod 1000003 —— 用小数核对模幂。
        let mut expected = 1_u128;
        for _ in 0..200 {
            expected = expected * 3 % 1_000_003;
        }
        assert_eq!(
            rsa::mod_pow(
                &Big::from_u64(3),
                &Big::from_u64(200),
                &Big::from_u64(1_000_003)
            ),
            Big::from_u64(expected as u64)
        );
    }

    #[test]
    fn decrypts_reference_vectors() {
        let key = test_key();
        let oaep = decode_b64url("mlI28jaNsstWoRTtOqY0QdUouHvsIBA7qqUXxUHPxrTzlWIopDP2mcuTF5U5xbY3gLidTGGy69Y__tboOPOQvrMJEVPGI3Hwr_RKgXAH-bEe7v7KIg6tEbOml11aQZ4eiMvtKAO8-QpFGoGswdf5_g9bmGznmMdC2l0hUseRpb-x143H8m4j8lp0tSylQaEMl8a-RtZftT2H9Oo0AAWKylJ1Kh9A6BBMkIIhRKqkyfPI2kV0ZrzDHQURlCdKpQxYTQKq53nLKYqbMf9TQqKfmLZtQttvMFtZCkxP7D8F_61nvwKHIchwvjO1S0-f2jKsFvC9bSMLEYfEIqfNccWuog==").unwrap();
        assert_eq!(key.decrypt(&oaep).unwrap(), b"secret-token");
        let legacy = decode_b64url("gG5PdyBrsz1FvLGwpLKExVqq0CmvzR0o6FkYvbM5NIeWBjtVOmI94c1RskDR5PgpYtv-OaE965CtljpZ6xzscwtdjxEFiGGV-jdEPiRtuSOsjguGIxzPwKsasvq-GXs9pFE6XPFjkNTWUl4tlx-QHq_kUwt8zJLF4udpYWx_PWDhZd-O7tEGcKkgeZ7Z0YTUqvLDlAUhPEHIxHQsPvVyV4mPUNHsR6RbF7VcWzv5mvKlemPvdfaNJO8aXuPvNu22dR1c6Z549VnzNj8onMZTHM7F166HU1WneGwDKtnhRnUbUqP-VmqwtlBhx9NLyxoqFf_-mcK1U2p0Sf73AWd6Gw==").unwrap();
        assert_eq!(key.decrypt(&legacy).unwrap(), b"old-token");
        // 公钥参数：PKCS#1 DER，带填充的 URL-safe base64。
        let der = key.public_der();
        assert_eq!(der.len(), 270);
        assert_eq!(
            base64::engine::general_purpose::URL_SAFE.encode(&der),
            "MIIBCgKCAQEAz8DBr-ZXGSx5VkX41akgSl6mCXC949-RqAA9SGdNQ3hI-M5tWb3qW6W6rgG7CFau-_VbSMkbgWbY1ggLkGrhq7Nsp0CZeZI1xjkJTZxP9UD5X2OQIUHdY_EXAC3faETKK94Ur4JKQPvB-jvi87255547ICIwcukVk-6bwngWiardKEiMqxNMXR9-gkd8767YTPww5dWJ51QAUtkzTrnzfshfF1vSVrEsKQYqa_WNPp55zIYnMC6c4RbMxHSXhu0AsbbBUSIU0cjGvHOMlpzBpTcxTWRakqJXjqpKP7ghi4SNVcYYZ8pzCD3nzn0vOnhlpFuDHmdIfSoSrLgPj9N7OwIDAQAB"
        );
        // 回调参数（带 %3D 的填充也认）。
        let mut params = std::collections::BTreeMap::new();
        params.insert("user_id".to_string(), "4242".to_string());
        params.insert(
            "access_token".to_string(),
            base64::engine::general_purpose::URL_SAFE.encode(&oaep),
        );
        assert_eq!(
            parse_callback(&params, &key).unwrap(),
            Some(("4242".to_string(), "secret-token".to_string()))
        );
        params.remove("user_id");
        assert_eq!(parse_callback(&params, &key).unwrap(), None);
    }

    #[test]
    fn generated_keys_round_trip() {
        let key = PrivateKey::generate(768, &mut editor_kit::random_bytes);
        assert_eq!(key.n.bits(), 768);
        let ciphertext = key.encrypt_oaep(b"hello", &[7; 32]);
        assert_eq!(key.decrypt(&ciphertext).unwrap(), b"hello");
        let prime = Big::from_u64(1_000_000_007);
        assert!(rsa::probably_prime(
            &prime,
            8,
            &mut editor_kit::random_bytes
        ));
        assert!(!rsa::probably_prime(
            &Big::from_u64(1_000_000_007 * 3),
            8,
            &mut editor_kit::random_bytes
        ));
        // 561 是 Carmichael 数。
        assert!(!rsa::probably_prime(
            &Big::from_be(&unhex("0000000000000231")),
            8,
            &mut editor_kit::random_bytes
        ));
    }

    #[test]
    fn organizations_are_picked_by_plan() {
        let me = json!({
            "user": { "github_login": "octo", "legacy_user_id": 4242 },
            "organizations": [
                { "id": "org-team", "name": "Team" },
                { "id": "org-personal", "name": "Me", "is_personal": true },
                { "id": "org-off" },
            ],
            "default_organization_id": "org-team",
            "plans_by_organization": { "org-personal": "zed_student", "org-off": "zed_pro" },
            "configuration_by_organization": { "org-off": { "is_zed_model_provider_enabled": false } },
        });
        assert_eq!(
            pick_org(&me, ""),
            ("org-personal".to_string(), "zed_student".to_string())
        );
        assert_eq!(
            pick_org(&me, "org-off"),
            ("org-personal".to_string(), "zed_student".to_string())
        );
        let none = json!({
            "organizations": [{ "id": "a" }, { "id": "b" }],
            "default_organization_id": "b",
            "plans_by_organization": {},
        });
        assert_eq!(pick_org(&none, "").0, "b");
        assert_eq!(pick_org(&none, "a").0, "a");
        assert_eq!(pick_org(&none, "gone").0, "b");
        assert_eq!(
            pick_org(&json!({ "plan": { "plan_v3": "zed_pro" } }), "").1,
            "zed_pro"
        );
        assert_eq!(account_name(&me), "octo");
        assert_eq!(
            account_name(&json!({ "user": { "legacy_user_id": 7 } })),
            "7"
        );
        assert_eq!(plan_label("zed_pro_trial"), "Pro Trial");
    }

    #[test]
    fn keychain_output_parses() {
        let output = "keychain: \"/Users/me/Library/Keychains/login.keychain-db\"\nclass: \"inet\"\nattributes:\n    \"acct\"<blob>=\"4242\"\n    \"srvr\"<blob>=\"https://zed.dev\"\n";
        assert_eq!(keychain_account(output).as_deref(), Some("4242"));
        assert_eq!(keychain_account("\"acct\"<blob>=<NULL>"), None);
    }

    #[test]
    fn models_parse_and_providers_guess() {
        let value = json!({"models":[
            {"provider":"anthropic","id":"claude-sonnet-4-5"},
            {"provider":"open_ai","id":"gpt-5"},
            {"provider":"google","id":"gemini-2.5-pro"},
            {"provider":"x_ai","id":"grok-4"},
            {"provider":"anthropic","id":"claude-old","is_disabled":true},
            {"provider":"mystery","id":"m-1"}
        ],"default_model":"gpt-5"});
        let models = parse_models(&value);
        assert_eq!(models.len(), 4);
        assert_eq!(models[0], ("gpt-5".to_string(), Provider::OpenAi));
        assert_eq!(Provider::guess("claude-x"), Provider::Anthropic);
        assert_eq!(Provider::guess("gemini-3"), Provider::Google);
        assert_eq!(Provider::guess("grok-5"), Provider::XAi);
        assert_eq!(Provider::guess("o9"), Provider::OpenAi);
    }

    fn tool_request(model: &str) -> Value {
        json!({
            "model": model,
            "max_tokens": 1000,
            "messages": [
                { "role": "developer", "content": "be brief" },
                { "role": "user", "content": "read a" },
                { "role": "assistant", "content": "ok", "reasoning_content": "r", "tool_calls": [
                    { "id": "call_1", "type": "function", "function": { "name": "read", "arguments": "{\"path\":\"a\"}" } },
                ] },
                { "role": "tool", "tool_call_id": "call_1", "content": "A" },
            ],
            "tools": [{ "type": "function", "function": { "name": "read", "description": "Read", "parameters": {
                "type": "object", "additionalProperties": false, "$schema": "x",
                "properties": { "path": { "type": ["string", "null"] }, "mode": { "enum": ["r", "w"] } },
                "required": ["path"],
            } } }],
        })
    }

    #[test]
    fn requests_per_provider() {
        let anthropic = completion_body(
            &tool_request("claude-sonnet-5"),
            "claude-sonnet-5",
            Provider::Anthropic,
        )
        .unwrap();
        assert_eq!(anthropic["provider"], "anthropic");
        let inner = &anthropic["provider_request"];
        assert!(inner.get("stream").is_none());
        assert_eq!(inner["system"], "be brief");
        assert!(inner.to_string().contains("\"is_error\":false"));

        let xai = completion_body(&tool_request("grok-4.7"), "grok-4.7", Provider::XAi).unwrap();
        let inner = &xai["provider_request"];
        assert_eq!(inner["max_completion_tokens"], 1000);
        assert!(inner.get("max_tokens").is_none());
        assert_eq!(inner["messages"][0]["role"], "system");
        assert!(inner["messages"][2].get("reasoning_content").is_none());
        assert_eq!(inner["stream"], true);

        let openai =
            completion_body(&tool_request("gpt-5.5"), "gpt-5.5", Provider::OpenAi).unwrap();
        let inner = &openai["provider_request"];
        let input = inner["input"].as_array().unwrap();
        assert_eq!(input[0]["role"], "system");
        assert_eq!(input[0]["content"][0]["type"], "input_text");
        assert_eq!(input[2]["content"][0]["type"], "output_text");
        assert_eq!(input[3]["type"], "function_call");
        assert_eq!(input[3]["call_id"], "call_1");
        assert_eq!(input[4]["type"], "function_call_output");
        assert_eq!(input[4]["output"], "A");
        assert_eq!(inner["tools"][0]["name"], "read");
        assert_eq!(inner["max_output_tokens"], 1000);

        let mut gemini_input = tool_request("gemini-3.5-flash");
        gemini_input["reasoning_effort"] = json!("high");
        let gemini = completion_body(&gemini_input, "gemini-3.5-flash", Provider::Google).unwrap();
        let inner = &gemini["provider_request"];
        assert_eq!(inner["model"], "models/gemini-3.5-flash");
        assert_eq!(inner["systemInstruction"]["parts"][0]["text"], "be brief");
        let contents = inner["contents"].as_array().unwrap();
        assert_eq!(contents[1]["role"], "model");
        assert_eq!(contents[1]["parts"][1]["functionCall"]["args"]["path"], "a");
        assert_eq!(contents[2]["parts"][0]["functionResponse"]["name"], "read");
        let parameters = &inner["tools"][0]["functionDeclarations"][0]["parameters"];
        assert!(parameters.get("additionalProperties").is_none());
        assert!(parameters.get("$schema").is_none());
        assert_eq!(
            parameters["properties"]["path"],
            json!({ "type": "string", "nullable": true })
        );
        assert_eq!(parameters["properties"]["mode"]["type"], "string");
        assert_eq!(inner["generationConfig"]["maxOutputTokens"], 1000);
        assert_eq!(
            inner["generationConfig"]["thinkingConfig"]["includeThoughts"],
            true
        );
    }

    fn run_lines(
        provider: Provider,
        wrapped: bool,
        lines: &[&str],
    ) -> (anyhow::Result<String>, bool) {
        let mut state = ZedStream {
            mapper: Mapper::new(provider),
            wrapped,
            ended: false,
        };
        let mut sse = ChatSse::new("m");
        let mut out = Vec::new();
        for line in lines {
            match state.push(&mut sse, line) {
                Ok(bytes) => out.extend(bytes),
                Err(error) => return (Err(error), state.ended),
            }
        }
        if !sse.is_finished() && (state.ended || !wrapped) {
            out.extend(state.mapper.finish(&mut sse));
        }
        (Ok(String::from_utf8(out).unwrap()), state.ended)
    }

    #[test]
    fn anthropic_lines_become_chat_sse() {
        let (text, ended) = run_lines(
            Provider::Anthropic,
            true,
            &[
                r#"{"status":"started"}"#,
                r#"{"status":{"queued":{"position":2}}}"#,
                r#"{"event":{"type":"message_start","message":{"usage":{"input_tokens":1,"output_tokens":0}}}}"#,
                r#"{"event":{"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}}"#,
                r#"{"event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"hi"}}}"#,
                r#"{"event":{"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":1}}}"#,
                r#"{"event":{"type":"message_stop"}}"#,
                r#"{"status":"stream_ended"}"#,
            ],
        );
        assert!(ended);
        let completion = aggregate(&text.unwrap(), "m").unwrap();
        assert_eq!(completion["choices"][0]["message"]["content"], "hi");
        assert_eq!(completion["usage"]["total_tokens"], 2);
    }

    #[test]
    fn responses_lines_become_chat_sse() {
        let (text, _) = run_lines(
            Provider::OpenAi,
            true,
            &[
                r#"{"event":{"type":"response.created","response":{"id":"r"}}}"#,
                r#"{"event":{"type":"response.reasoning_summary_text.delta","delta":"think"}}"#,
                r#"{"event":{"type":"response.output_text.delta","item_id":"i","output_index":0,"content_index":0,"delta":"Hello from GPT"}}"#,
                r#"{"event":{"type":"response.output_item.added","item":{"type":"function_call","id":"fc_1","call_id":"call_9","name":"read","arguments":""}}}"#,
                r#"{"event":{"type":"response.function_call_arguments.delta","item_id":"fc_1","delta":"{\"p\":1}"}}"#,
                r#"{"event":{"type":"response.output_item.done","item":{"type":"function_call","id":"fc_1","call_id":"call_9","name":"read","arguments":"{\"p\":1}"}}}"#,
                r#"{"event":{"type":"response.completed","response":{"id":"r","status":"completed","usage":{"input_tokens":5,"output_tokens":3}}}}"#,
                r#"{"status":"stream_ended"}"#,
            ],
        );
        let completion = aggregate(&text.unwrap(), "m").unwrap();
        let message = &completion["choices"][0]["message"];
        assert_eq!(message["content"], "Hello from GPT");
        assert_eq!(message["reasoning_content"], "think");
        assert_eq!(message["tool_calls"][0]["id"], "call_9");
        assert_eq!(
            message["tool_calls"][0]["function"]["arguments"],
            "{\"p\":1}"
        );
        assert_eq!(completion["choices"][0]["finish_reason"], "tool_calls");
        assert_eq!(completion["usage"]["total_tokens"], 8);
    }

    #[test]
    fn chat_and_gemini_lines_become_chat_sse() {
        let (text, _) = run_lines(
            Provider::XAi,
            true,
            &[
                r#"{"event":{"id":"c","object":"chat.completion.chunk","choices":[{"index":0,"delta":{"role":"assistant","content":"Hello from Grok"}}]}}"#,
                r#"{"event":{"id":"c","object":"chat.completion.chunk","choices":[{"index":0,"delta":{},"finish_reason":"stop"}],"usage":{"prompt_tokens":5,"completion_tokens":3}}}"#,
                r#"{"status":"stream_ended"}"#,
            ],
        );
        let completion = aggregate(&text.unwrap(), "m").unwrap();
        assert_eq!(
            completion["choices"][0]["message"]["content"],
            "Hello from Grok"
        );
        assert_eq!(completion["usage"]["total_tokens"], 8);

        let (text, _) = run_lines(
            Provider::Google,
            true,
            &[
                r#"{"event":{"candidates":[{"content":{"role":"model","parts":[{"text":"hm","thought":true},{"text":"Hello from Gemini"}]}}]}}"#,
                r#"{"event":{"candidates":[{"content":{"role":"model","parts":[{"functionCall":{"name":"read","args":{"p":"a"}}}]},"finishReason":"STOP"}],"usageMetadata":{"promptTokenCount":5,"candidatesTokenCount":3,"thoughtsTokenCount":1}}}"#,
                r#"{"status":"stream_ended"}"#,
            ],
        );
        let completion = aggregate(&text.unwrap(), "m").unwrap();
        let message = &completion["choices"][0]["message"];
        assert_eq!(message["content"], "Hello from Gemini");
        assert_eq!(message["reasoning_content"], "hm");
        assert_eq!(
            message["tool_calls"][0]["function"]["arguments"],
            "{\"p\":\"a\"}"
        );
        assert_eq!(completion["choices"][0]["finish_reason"], "tool_calls");
        assert_eq!(
            completion["usage"]["completion_tokens_details"]["reasoning_tokens"],
            1
        );
    }

    #[test]
    fn failures_keep_status() {
        let (result, _) = run_lines(
            Provider::Anthropic,
            true,
            &[r#"{"status":{"failed":{"code":"upstream_http_429","message":"Rate limited"}}}"#],
        );
        assert_eq!(status_of(&result.unwrap_err()), 429);
        let (result, _) = run_lines(
            Provider::Anthropic,
            true,
            &[r#"{"status":{"failed":{"code":"billing_limit","message":""}}}"#],
        );
        let error = result.unwrap_err();
        assert_eq!(status_of(&error), 402);
        assert!(error.to_string().contains("billing_limit"));
        let (result, _) = run_lines(
            Provider::Anthropic,
            true,
            &[r#"{"status":{"failed":{"code":"http_999"}}}"#],
        );
        assert_eq!(status_of(&result.unwrap_err()), 502);
        let error = http_failure(
            500,
            r#"{"code":"upstream_http_529","message":"Overloaded","upstream_status":529,"retry_after":2}"#,
        );
        assert_eq!(status_of(&error), 529);
        assert!(error.to_string().contains("Overloaded"));
        assert_eq!(status_of(&http_failure(402, "")), 402);
        assert_eq!(
            status_of(&http_failure(403, r#"{"message":"sign in again later"}"#)),
            403
        );
        // 没有包装层（老服务端）：裸事件也能读。
        let (text, _) = run_lines(
            Provider::XAi,
            false,
            &[r#"{"choices":[{"index":0,"delta":{"content":"plain"}}]}"#],
        );
        assert!(text.unwrap().contains("plain"));
    }

    #[tokio::test]
    async fn cut_short_streams_end_with_a_note_or_502() {
        let stream = editor_kit::spawn_chat("m", |mut out| async move {
            let mut state = ZedStream {
                mapper: Mapper::new(Provider::XAi),
                wrapped: true,
                ended: false,
            };
            handle_line(
                &mut out,
                &mut state,
                r#"{"event":{"choices":[{"index":0,"delta":{"content":"half"}}]}}"#,
            )
            .await;
            cut_short(&mut out, &mut state, None).await;
        });
        let text: String = stream
            .map(|item| String::from_utf8(item.unwrap()).unwrap())
            .collect::<Vec<_>>()
            .await
            .concat();
        assert!(text.contains("回复被截断"));
        assert!(text.ends_with("data: [DONE]\n\n"));

        let stream = editor_kit::spawn_chat("m", |mut out| async move {
            let mut state = ZedStream {
                mapper: Mapper::new(Provider::XAi),
                wrapped: true,
                ended: false,
            };
            cut_short(&mut out, &mut state, None).await;
        });
        let items: Vec<_> = stream.collect().await;
        assert_eq!(status_of(items[0].as_ref().unwrap_err()), 502);
    }
}
