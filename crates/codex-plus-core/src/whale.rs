//! 鲸鱼挂件的独立余额观测。凭据只在后端使用，账本只保存账户指纹与金额。
use std::collections::{BTreeMap, HashSet};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde::Serialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio::sync::Mutex;

use crate::settings::{BackendSettings, RelayMode};

const BALANCE_URL: &str = "https://api.deepseek.com/user/balance";
const CACHE_TTL: Duration = Duration::from_secs(60);
const MONEY_SCALE: f64 = 100_000_000.0;
const MAX_AMOUNT: f64 = 100_000_000.0;
const MAX_BODY: usize = 64 * 1024;

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct Balance {
    currency: String,
    total: f64,
    granted: Option<f64>,
    topped_up: Option<f64>,
    observed_today: f64,
}

struct CacheEntry {
    checked_at: Instant,
    day: String,
    value: Value,
    last_success: Option<Value>,
}

#[derive(Clone)]
struct BalanceRequest {
    url: String,
    field: Option<Vec<String>>,
    currency: String,
    scale: f64,
}

impl BalanceRequest {
    fn scope(&self) -> String {
        json!([self.url, self.field, self.currency, self.scale]).to_string()
    }
}

// 锁覆盖请求和落账，同一进程的并发刷新只会查询一次；也避免多窗口同时重复记账。
static CACHE: LazyLock<Mutex<BTreeMap<String, CacheEntry>>> =
    LazyLock::new(|| Mutex::new(BTreeMap::new()));

pub fn enabled(settings: &BackendSettings) -> bool {
    settings.enhancements_enabled && settings.codex_app_whale_widget_enabled
}

fn empty_status(status: &str, message: Option<&str>) -> Value {
    let mut value = json!({
        "status": status,
        "provider": {"id": "", "name": "", "accountId": ""},
        "balances": [],
        "updatedAt": 0,
        "stale": false,
    });
    if let Some(message) = message {
        value["message"] = json!(message);
    }
    value
}

pub fn unavailable(message: &str) -> Value {
    empty_status("unavailable", Some(message))
}

pub async fn balance(settings: &BackendSettings, ledger_path: PathBuf) -> Value {
    balance_with_fetch(settings, ledger_path, |key, request| async move {
        let value = fetch_balance_json(&request.url, &key).await?;
        if request.field.is_some() {
            parse_custom_balance(&value, &request)
        } else {
            parse_balances(&value)
        }
    })
    .await
}

async fn balance_with_fetch<F, Fut>(
    settings: &BackendSettings,
    ledger_path: PathBuf,
    fetch: F,
) -> Value
where
    F: FnOnce(String, BalanceRequest) -> Fut,
    Fut: Future<Output = Result<Vec<Balance>, &'static str>>,
{
    // 开关关闭时不取凭据、不访问缓存/账本，也不发网络请求。
    if !enabled(settings) {
        return empty_status("disabled", None);
    }
    if settings.codex_app_whale_balance_protocol == "off" {
        return empty_status("disabled", Some("余额查询已关闭，会话用量仍可查看"));
    }
    let profile = settings.active_relay_profile();
    let provider = json!({"id": profile.id, "name": profile.name, "accountId": ""});
    let with_provider = |mut value: Value| {
        value["provider"] = provider.clone();
        value
    };
    let base_url = crate::relay_config::relay_profile_base_url(&profile);
    if !settings.relay_profiles_enabled || profile.relay_mode == RelayMode::Aggregate {
        return with_provider(empty_status(
            "unsupported",
            Some("当前供应商未配置余额接口，可在增强配置中设置"),
        ));
    }
    let request = if settings.codex_app_whale_balance_protocol == "custom" {
        match custom_request(settings, &base_url) {
            Ok(request) => request,
            Err(message) => return with_provider(unavailable(message)),
        }
    } else if is_deepseek_origin(&base_url) {
        BalanceRequest {
            url: BALANCE_URL.to_string(),
            field: None,
            currency: String::new(),
            scale: 1.0,
        }
    } else {
        return with_provider(empty_status(
            "unsupported",
            Some("当前供应商未配置余额接口，可在增强配置中设置"),
        ));
    };
    let key = crate::relay_config::relay_profile_api_key(&profile);
    if profile.uses_no_auth() || key.is_empty() {
        return with_provider(unavailable("当前供应商未配置可用的 API 密钥"));
    }
    // 同源账户共用基准，换 key 或金额映射规则则独立记账。
    let account = account_fingerprint(&key, &request.scope());
    let with_provider = |value| {
        let mut value = with_provider(value);
        value["provider"]["accountId"] = json!(account);
        value
    };
    let cache_key = format!("{}:{account}", ledger_path.to_string_lossy());
    let day = match local_day() {
        Ok(day) => day,
        Err(_) => return with_provider(unavailable("无法确定本地日期")),
    };
    let mut cache = CACHE.lock().await;
    if let Some(entry) = cache.get(&cache_key) {
        if entry.checked_at.elapsed() < CACHE_TTL && entry.day == day {
            return with_provider(entry.value.clone());
        }
    }
    let fetched = fetch(key, request).await;
    let now = now_ms();
    // 网络等待可能跨越午夜，按实际收到样本时的本地日期落账。
    let day = local_day().unwrap_or(day);
    let previous = cache.get(&cache_key).and_then(|entry| {
        entry.last_success.clone().map(|mut value| {
            if entry.day != day {
                if let Some(balances) = value["balances"].as_array_mut() {
                    for balance in balances {
                        balance["observedToday"] = json!(0);
                    }
                }
            }
            value
        })
    });
    let result = match fetched {
        Ok(balances) => {
            let ledger_day = day.clone();
            let ledger_account = account.clone();
            tokio::task::spawn_blocking(move || {
                record_balances(&ledger_path, &ledger_account, &ledger_day, now, balances)
            })
            .await
            .map_err(|_| "余额观测账本暂不可用，未更新消费统计")
            .and_then(|result| result.map_err(|_| "余额观测账本暂不可用，未更新消费统计"))
        }
        Err(message) => Err(message),
    };
    let (value, last_success) = match result {
        Ok(balances) => {
            let value = json!({
                "status": "ok", "balances": balances, "updatedAt": now, "stale": false,
            });
            (value.clone(), Some(value))
        }
        Err(message) => {
            let mut value = previous.clone().unwrap_or_else(|| unavailable(message));
            value["status"] = json!("unavailable");
            value["stale"] = json!(previous.is_some());
            value["message"] = json!(message);
            (value, previous)
        }
    };
    if cache.len() >= 64 && !cache.contains_key(&cache_key) {
        if let Some(oldest) = cache
            .iter()
            .min_by_key(|(_, entry)| entry.checked_at)
            .map(|(key, _)| key.clone())
        {
            cache.remove(&oldest);
        }
    }
    cache.insert(
        cache_key,
        CacheEntry {
            checked_at: Instant::now(),
            day,
            value: value.clone(),
            last_success,
        },
    );
    with_provider(value)
}

fn is_deepseek_origin(raw: &str) -> bool {
    reqwest::Url::parse(raw.trim()).is_ok_and(|url| {
        url.scheme() == "https"
            && url.host_str() == Some("api.deepseek.com")
            && url.port_or_known_default() == Some(443)
            && url.username().is_empty()
            && url.password().is_none()
            && url.query().is_none()
            && url.fragment().is_none()
    })
}

fn custom_request(
    settings: &BackendSettings,
    base_url: &str,
) -> Result<BalanceRequest, &'static str> {
    let base = reqwest::Url::parse(base_url).map_err(|_| "供应商地址无效")?;
    if base.scheme() != "https"
        || base.host_str().is_none()
        || !base.username().is_empty()
        || base.password().is_some()
        || base.query().is_some()
        || base.fragment().is_some()
    {
        return Err("自定义余额接口需要不含凭据参数的 HTTPS 供应商地址");
    }
    let path = settings.codex_app_whale_balance_path.trim();
    if path.is_empty()
        || path.len() > 2048
        || path.starts_with("//")
        || path
            .chars()
            .any(|c| matches!(c, '?' | '#' | '\\' | ':') || c.is_control() || c.is_whitespace())
    {
        return Err("余额接口必须是同源相对路径，不能包含网址、查询参数或片段");
    }
    let path = format!("/{}", path.trim_start_matches('/'));
    let url = base.join(&path).map_err(|_| "余额接口路径无效")?;
    if url.origin() != base.origin() || url.query().is_some() || url.fragment().is_some() {
        return Err("余额接口必须与当前供应商同源");
    }
    let field = field_tokens(settings.codex_app_whale_balance_field.trim())
        .ok_or("请配置有效的余额 JSON 字段路径")?;
    let currency = settings.codex_app_whale_balance_currency.trim();
    if currency.len() != 3 || !currency.bytes().all(|byte| byte.is_ascii_uppercase()) {
        return Err("余额币种必须是三位大写代码");
    }
    let scale = settings.codex_app_whale_balance_scale;
    if !scale.is_finite() || scale <= 0.0 {
        return Err("余额倍率必须是有限正数");
    }
    Ok(BalanceRequest {
        url: url.to_string(),
        field: Some(field),
        currency: currency.to_string(),
        scale,
    })
}

fn field_tokens(field: &str) -> Option<Vec<String>> {
    if field.is_empty() || field.len() > 256 {
        return None;
    }
    let mut rest = field;
    let mut tokens = Vec::new();
    while !rest.is_empty() {
        if rest.starts_with('[') {
            let end = rest.find(']')?;
            let number = &rest[1..end];
            if number.is_empty() || !number.bytes().all(|byte| byte.is_ascii_digit()) {
                return None;
            }
            tokens.push(number.to_string());
            rest = &rest[end + 1..];
        } else {
            let end = rest.find(['.', '[', ']']).unwrap_or(rest.len());
            let name = &rest[..end];
            if name.is_empty()
                || !name
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '$'))
            {
                return None;
            }
            tokens.push(name.to_string());
            rest = &rest[end..];
        }
        if tokens.len() > 32 {
            return None;
        }
        if let Some(next) = rest.strip_prefix('.') {
            if next.is_empty() || next.starts_with('[') {
                return None;
            }
            rest = next;
        } else if !rest.is_empty() && !rest.starts_with('[') {
            return None;
        }
    }
    Some(tokens)
}

fn parse_custom_balance(
    value: &Value,
    request: &BalanceRequest,
) -> Result<Vec<Balance>, &'static str> {
    let mut selected = value;
    for token in request.field.as_ref().ok_or("余额字段未配置")? {
        selected = if selected.is_array() {
            token
                .parse::<usize>()
                .ok()
                .and_then(|index| selected.get(index))
        } else {
            selected.get(token)
        }
        .ok_or("响应中未找到配置的余额字段")?;
    }
    // 先校验原数值，再缩放并量化，不能在缩放之前丢掉小数。
    let raw = selected
        .as_f64()
        .or_else(|| selected.as_str()?.parse::<f64>().ok())
        .filter(|value| value.is_finite() && *value >= 0.0)
        .ok_or("配置的余额字段不是有限非负数")?;
    let total = amount(&json!(raw * request.scale))
        .filter(|value| *value >= 0.0)
        .ok_or("余额换算结果超出可用范围")?;
    Ok(vec![Balance {
        currency: request.currency.clone(),
        total,
        granted: None,
        topped_up: None,
        observed_today: 0.0,
    }])
}

fn account_fingerprint(key: &str, scope: &str) -> String {
    let mut hash = Sha256::new();
    hash.update(b"codex-plus-whale:v1\0");
    hash.update(scope.as_bytes());
    hash.update(b"\0");
    hash.update(key.as_bytes());
    format!("{:x}", hash.finalize())
}

async fn fetch_balance_json(url: &str, key: &str) -> Result<Value, &'static str> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(8))
        .connect_timeout(Duration::from_secs(4))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| "余额查询暂不可用")?;
    let mut response = client
        .get(url)
        .bearer_auth(key)
        .send()
        .await
        .map_err(|_| "余额查询失败，请检查网络连接")?;
    if !response.status().is_success() {
        return Err(match response.status().as_u16() {
            401 | 403 => "余额查询鉴权失败，请检查供应商密钥",
            429 => "余额查询频率受限，请稍后重试",
            _ => "供应商暂时无法提供余额",
        });
    }
    if response
        .content_length()
        .is_some_and(|length| length > MAX_BODY as u64)
    {
        return Err("余额响应格式无效");
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| "余额响应读取失败")? {
        if bytes.len().saturating_add(chunk.len()) > MAX_BODY {
            return Err("余额响应格式无效");
        }
        bytes.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&bytes).map_err(|_| "余额响应格式无效")
}

fn amount(value: &Value) -> Option<f64> {
    let amount = value.as_f64().or_else(|| value.as_str()?.parse().ok())?;
    (amount.is_finite() && amount.abs() <= MAX_AMOUNT)
        .then(|| (amount * MONEY_SCALE).round() / MONEY_SCALE)
}

fn parse_balances(value: &Value) -> Result<Vec<Balance>, &'static str> {
    let rows = value["balance_infos"]
        .as_array()
        .ok_or("余额响应格式无效")?;
    if rows.is_empty() || rows.len() > 8 {
        return Err("余额响应格式无效");
    }
    let mut currencies = HashSet::new();
    rows.iter()
        .map(|row| {
            let currency = row["currency"]
                .as_str()
                .filter(|currency| matches!(*currency, "CNY" | "USD"))
                .ok_or("余额响应币种无效")?;
            if !currencies.insert(currency) {
                return Err("余额响应币种重复");
            }
            let optional_amount = |key: &str| -> Result<Option<f64>, &'static str> {
                match row.get(key).filter(|value| !value.is_null()) {
                    None => Ok(None),
                    Some(value) => amount(value).map(Some).ok_or("余额响应金额无效"),
                }
            };
            Ok(Balance {
                currency: currency.to_string(),
                total: amount(&row["total_balance"]).ok_or("余额响应金额无效")?,
                granted: optional_amount("granted_balance")?,
                topped_up: optional_amount("topped_up_balance")?,
                observed_today: 0.0,
            })
        })
        .collect()
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn local_day() -> rusqlite::Result<String> {
    Connection::open_in_memory()?.query_row("SELECT date('now', 'localtime')", [], |row| row.get(0))
}

fn record_balances(
    path: &Path,
    account: &str,
    day: &str,
    now: u64,
    mut balances: Vec<Balance>,
) -> anyhow::Result<Vec<Balance>> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut connection = Connection::open(path)?;
    connection.busy_timeout(Duration::from_secs(2))?;
    let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    tx.execute_batch(
        "CREATE TABLE IF NOT EXISTS whale_balance_observations (
            account TEXT NOT NULL, currency TEXT NOT NULL, day TEXT NOT NULL,
            total_units INTEGER NOT NULL, observed_units INTEGER NOT NULL,
            updated_at INTEGER NOT NULL, PRIMARY KEY (account, currency)
        );",
    )?;
    for balance in &mut balances {
        let previous: Option<(String, i64, i64, u64)> = tx.query_row(
            "SELECT day, total_units, observed_units, updated_at FROM whale_balance_observations
             WHERE account = ?1 AND currency = ?2",
            params![account, balance.currency],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        ).optional()?;
        let total_units = (balance.total * MONEY_SCALE).round() as i64;
        // 跨进程请求或系统时钟回拨可能让旧样本后到。整批回滚，不能把未落账
        // 的旧余额与已落账的新消费合并成一个成功响应。
        if previous
            .as_ref()
            .is_some_and(|(_, _, _, updated)| *updated > now)
        {
            anyhow::bail!("balance observation is older than the committed baseline");
        }
        let observed_units = previous
            .as_ref()
            .filter(|(old_day, _, _, _)| old_day == day)
            .map(|(_, old_total, observed, _)| {
                observed.saturating_add(old_total.saturating_sub(total_units).max(0))
            })
            .unwrap_or(0)
            .max(0);
        balance.observed_today = observed_units as f64 / MONEY_SCALE;
        tx.execute(
            "INSERT INTO whale_balance_observations
             (account, currency, day, total_units, observed_units, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT (account, currency) DO UPDATE SET day=excluded.day,
             total_units=excluded.total_units, observed_units=excluded.observed_units,
             updated_at=excluded.updated_at",
            params![
                account,
                balance.currency,
                day,
                total_units,
                observed_units,
                now
            ],
        )?;
    }
    tx.commit()?;
    Ok(balances)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn settings() -> BackendSettings {
        let mut settings = BackendSettings::default();
        settings.codex_app_whale_widget_enabled = true;
        settings.active_relay_id = "deepseek-test".into();
        settings.relay_profiles = vec![crate::settings::RelayProfile {
            id: "deepseek-test".into(),
            name: "DeepSeek".into(),
            base_url: "https://api.deepseek.com/v1".into(),
            api_key: "fake-whale-key".into(),
            ..crate::settings::RelayProfile::default()
        }];
        settings
    }

    fn sample(currency: &str, total: f64) -> Balance {
        Balance {
            currency: currency.into(),
            total,
            granted: None,
            topped_up: None,
            observed_today: 0.0,
        }
    }

    #[test]
    fn official_origin_is_strict_and_never_derived_from_model_names() {
        assert!(is_deepseek_origin("https://api.deepseek.com/v1"));
        assert!(is_deepseek_origin("https://api.deepseek.com/beta"));
        for value in [
            "http://api.deepseek.com",
            "https://api.deepseek.com.evil.test",
            "https://deepseek.com",
            "https://api.deepseek.com:8443",
            "https://user@api.deepseek.com",
            "https://api.deepseek.com/?key=secret",
            "https://api.deepseek.com/#secret",
        ] {
            assert!(!is_deepseek_origin(value), "{value}");
        }
    }

    #[test]
    fn balance_parser_preserves_unknown_breakdowns_and_rejects_invalid_amounts() {
        let parsed = parse_balances(
            &json!({"balance_infos": [{"currency":"CNY", "total_balance":"10.12345678"}]}),
        )
        .unwrap();
        assert_eq!(parsed[0].total, 10.12345678);
        assert!(parsed[0].granted.is_none());
        assert!(parsed[0].topped_up.is_none());
        for value in [
            json!("NaN"),
            json!("inf"),
            json!("-inf"),
            json!("1e100"),
            json!(null),
            json!(true),
        ] {
            assert!(
                parse_balances(
                    &json!({"balance_infos":[{"currency":"CNY", "total_balance":value}]})
                )
                .is_err()
            );
        }
        assert!(
            parse_balances(&json!({"balance_infos": [
                {"currency":"CNY", "total_balance":"1"}, {"currency":"CNY", "total_balance":"2"}
            ]}))
            .is_err()
        );
    }

    fn custom_settings() -> BackendSettings {
        let mut settings = settings();
        settings.relay_profiles[0].base_url = "https://relay.example/v1".into();
        settings.codex_app_whale_balance_protocol = "custom".into();
        settings.codex_app_whale_balance_path = "/api/balance".into();
        settings.codex_app_whale_balance_field = "data.accounts[0].balance".into();
        settings.codex_app_whale_balance_currency = "USD".into();
        settings.codex_app_whale_balance_scale = 0.01;
        settings
    }

    #[test]
    fn custom_mapping_scales_only_explicit_numeric_fields_and_rejects_missing_values() {
        let settings = custom_settings();
        let request = custom_request(&settings, "https://relay.example/v1").unwrap();
        assert_eq!(request.url, "https://relay.example/api/balance");
        let result = parse_custom_balance(
            &json!({"data":{"accounts":[{"balance":"1234.56789"}]}}),
            &request,
        )
        .unwrap();
        assert_eq!(result[0].total, 12.3456789);
        assert_eq!(result[0].currency, "USD");
        assert!(result[0].granted.is_none());
        for value in [
            json!(null),
            json!("NaN"),
            json!("-1"),
            json!(true),
            json!("unknown"),
        ] {
            assert!(
                parse_custom_balance(&json!({"data":{"accounts":[{"balance":value}]}}), &request)
                    .is_err()
            );
        }
        assert!(parse_custom_balance(&json!({"data":{"accounts":[]}}), &request).is_err());
        assert_eq!(
            field_tokens("data.accounts.0.balance"),
            field_tokens("data.accounts[0].balance")
        );
        for field in [
            "",
            "data..balance",
            "data[",
            "data[-1]",
            "data[0]balance",
            "data.",
            "data[]",
        ] {
            assert!(field_tokens(field).is_none(), "{field}");
        }
    }

    #[test]
    fn custom_endpoints_cannot_escape_selected_provider_origin_or_carry_url_credentials() {
        let mut settings = custom_settings();
        for path in [
            "https://other.example/balance",
            "//other.example/balance",
            "\\other.example",
            "/balance?key=secret",
            "/balance#secret",
            "/bal\nance",
            "",
        ] {
            settings.codex_app_whale_balance_path = path.into();
            assert!(
                custom_request(&settings, "https://relay.example/v1").is_err(),
                "{path}"
            );
        }
        settings.codex_app_whale_balance_path = "api/balance".into();
        assert_eq!(
            custom_request(&settings, "https://relay.example/v1")
                .unwrap()
                .url,
            "https://relay.example/api/balance"
        );
        for base in [
            "http://relay.example",
            "https://user:pass@relay.example",
            "https://relay.example?key=secret",
            "https://relay.example/#token",
        ] {
            assert!(custom_request(&settings, base).is_err());
        }
        settings.codex_app_whale_balance_currency = "US Dollars".into();
        assert!(custom_request(&settings, "https://relay.example").is_err());
    }

    #[tokio::test]
    async fn custom_scope_changes_account_identity_and_restarts_observation_baseline() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("ledger.sqlite");
        let mut settings = custom_settings();
        let first = balance_with_fetch(&settings, path.clone(), |key, request| async move {
            assert_eq!(key, "fake-whale-key");
            assert_eq!(request.url, "https://relay.example/api/balance");
            Ok(vec![sample("USD", 10.0)])
        })
        .await;
        assert_eq!(first["status"], "ok");
        for mutation in 0..4 {
            match mutation {
                0 => settings.codex_app_whale_balance_scale = 0.02,
                1 => settings.codex_app_whale_balance_currency = "CNY".into(),
                2 => settings.codex_app_whale_balance_field = "data.balance".into(),
                _ => settings.relay_profiles[0].api_key = "fake-replacement-key".into(),
            }
            let next = balance_with_fetch(&settings, path.clone(), |_, request| async move {
                Ok(vec![sample(&request.currency, 1.0)])
            })
            .await;
            assert_ne!(
                next["provider"]["accountId"],
                first["provider"]["accountId"]
            );
            assert_eq!(next["balances"][0]["observedToday"], 0.0);
        }
        settings.codex_app_whale_balance_protocol = "off".into();
        let result = balance_with_fetch(&settings, path, |_, _| async {
            panic!("off must not fetch")
        })
        .await;
        assert_eq!(result["status"], "disabled");
        assert!(enabled(&settings));
    }

    #[test]
    fn ledger_records_only_drops_and_resets_each_local_day() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("ledger.sqlite");
        let observe = |day: &str, now: u64, total: f64| {
            record_balances(&path, "account", day, now, vec![sample("CNY", total)]).unwrap()[0]
                .observed_today
        };
        assert_eq!(observe("2026-10-08", 1, 10.0), 0.0);
        assert_eq!(observe("2026-10-08", 2, 9.9), 0.1);
        assert_eq!(observe("2026-10-08", 3, 9.9), 0.1);
        assert_eq!(observe("2026-10-08", 4, 20.0), 0.1);
        assert_eq!(observe("2026-10-08", 5, 19.99999999), 0.10000001);
        assert_eq!(observe("2026-10-09", 6, 19.0), 0.0);
        assert_eq!(observe("2026-10-09", 7, 18.0), 1.0);
        // 更旧的并发结果不覆盖新基准。
        assert!(
            record_balances(&path, "account", "2026-10-09", 6, vec![sample("CNY", 20.0)]).is_err()
        );
        assert_eq!(observe("2026-10-09", 8, 17.0), 2.0);
    }

    #[test]
    fn ledger_is_persistent_and_isolated_by_account_and_currency() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("ledger.sqlite");
        let first = account_fingerprint("fake-key-one", "test");
        let second = account_fingerprint("fake-key-two", "test");
        assert_ne!(first, second);
        record_balances(
            &path,
            &first,
            "2026-10-08",
            1,
            vec![sample("CNY", 10.0), sample("USD", 5.0)],
        )
        .unwrap();
        let result = record_balances(
            &path,
            &first,
            "2026-10-08",
            2,
            vec![sample("CNY", 9.0), sample("USD", 4.9)],
        )
        .unwrap();
        assert_eq!(result[0].observed_today, 1.0);
        assert_eq!(result[1].observed_today, 0.1);
        assert_eq!(
            record_balances(&path, &second, "2026-10-08", 3, vec![sample("CNY", 1.0)]).unwrap()[0]
                .observed_today,
            0.0
        );
        let bytes = std::fs::read(&path).unwrap();
        assert!(!String::from_utf8_lossy(&bytes).contains("fake-key"));
    }

    #[test]
    fn ledger_rejects_older_samples_atomically_across_currencies() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("ledger.sqlite");
        record_balances(&path, "account", "2026-10-08", 1, vec![sample("CNY", 10.0)]).unwrap();
        record_balances(&path, "account", "2026-10-08", 3, vec![sample("USD", 20.0)]).unwrap();
        // 第一币种本可更新，第二币种已存在新基准，必须让整批更新回滚。
        assert!(
            record_balances(
                &path,
                "account",
                "2026-10-08",
                2,
                vec![sample("CNY", 8.0), sample("USD", 18.0)]
            )
            .is_err()
        );
        let result = record_balances(
            &path,
            "account",
            "2026-10-08",
            4,
            vec![sample("CNY", 9.0), sample("USD", 19.0)],
        )
        .unwrap();
        assert_eq!(result[0].observed_today, 1.0);
        assert_eq!(result[1].observed_today, 1.0);
    }

    #[tokio::test]
    async fn disabled_and_unsupported_balances_do_not_fetch_or_create_ledger() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("missing").join("ledger.sqlite");
        let mut settings = settings();
        for (enhancements, whale) in [(false, true), (true, false)] {
            settings.enhancements_enabled = enhancements;
            settings.codex_app_whale_widget_enabled = whale;
            let response = balance_with_fetch(&settings, path.clone(), |_, _| async {
                panic!("disabled fetch")
            })
            .await;
            assert_eq!(response["status"], "disabled");
        }
        settings.enhancements_enabled = true;
        settings.codex_app_whale_widget_enabled = true;
        settings.relay_profiles[0].base_url = "https://relay.example/v1".into();
        settings.relay_profiles[0].model = "deepseek-chat".into();
        let response = balance_with_fetch(&settings, path.clone(), |_, _| async {
            panic!("unsupported fetch")
        })
        .await;
        assert_eq!(response["status"], "unsupported");
        assert!(!path.parent().unwrap().exists());
    }

    #[tokio::test]
    async fn concurrent_requests_share_one_fetch_and_do_not_expose_keys() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("ledger.sqlite");
        let settings = settings();
        let calls = Arc::new(AtomicUsize::new(0));
        let fetch = || {
            let calls = Arc::clone(&calls);
            move |_, _| async move {
                calls.fetch_add(1, Ordering::SeqCst);
                tokio::time::sleep(Duration::from_millis(20)).await;
                Ok(vec![sample("CNY", 10.0)])
            }
        };
        let (first, second) = tokio::join!(
            balance_with_fetch(&settings, path.clone(), fetch()),
            balance_with_fetch(&settings, path, fetch()),
        );
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(first, second);
        assert_eq!(first["status"], "ok");
        assert_eq!(first["balances"][0]["granted"], Value::Null);
        assert!(!first.to_string().contains("fake-whale-key"));
    }

    #[tokio::test]
    async fn failed_refresh_retains_stale_balance_and_failed_ledger_is_visible() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("ledger.sqlite");
        let settings = settings();
        let first = balance_with_fetch(&settings, path.clone(), |_, _| async {
            Ok(vec![sample("CNY", 10.0)])
        })
        .await;
        let cache_key = format!(
            "{}:{}",
            path.to_string_lossy(),
            account_fingerprint(
                "fake-whale-key",
                &BalanceRequest {
                    url: BALANCE_URL.into(),
                    field: None,
                    currency: String::new(),
                    scale: 1.0
                }
                .scope()
            )
        );
        CACHE.lock().await.get_mut(&cache_key).unwrap().checked_at = Instant::now() - CACHE_TTL;
        let second = balance_with_fetch(&settings, path, |_, _| async {
            Err("余额查询失败，请检查网络连接")
        })
        .await;
        assert_eq!(second["status"], "unavailable");
        assert_eq!(second["stale"], true);
        assert_eq!(second["balances"], first["balances"]);
        assert_eq!(second["updatedAt"], first["updatedAt"]);
        let invalid_path = temp.path().join("not-a-directory");
        std::fs::write(&invalid_path, "test").unwrap();
        let result = balance_with_fetch(
            &settings,
            invalid_path.join("ledger.sqlite"),
            |_, _| async { Ok(vec![sample("CNY", 10.0)]) },
        )
        .await;
        assert_eq!(result["status"], "unavailable");
        assert!(result["balances"].as_array().unwrap().is_empty());
        assert!(result["message"].as_str().unwrap().contains("账本"));
        assert!(!result.to_string().contains("not-a-directory"));
    }

    #[tokio::test]
    async fn balance_http_rejects_redirects_and_never_echoes_error_bodies() {
        use wiremock::{
            Mock, MockServer, ResponseTemplate,
            matchers::{method, path},
        };
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/redirect"))
            .respond_with(
                ResponseTemplate::new(302)
                    .insert_header("Location", format!("{}/target", server.uri())),
            )
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(path("/target"))
            .respond_with(ResponseTemplate::new(200))
            .expect(0)
            .mount(&server)
            .await;
        assert!(
            fetch_balance_json(&format!("{}/redirect", server.uri()), "fake-key")
                .await
                .is_err()
        );
        Mock::given(path("/error"))
            .respond_with(ResponseTemplate::new(401).set_body_string("fake-key secret error"))
            .mount(&server)
            .await;
        let error = fetch_balance_json(&format!("{}/error", server.uri()), "fake-key")
            .await
            .err()
            .unwrap();
        assert!(!error.contains("fake-key"));
        assert!(!error.contains(&server.uri()));
    }
}
