//! 原版挂件的内部兼容接口。图库与设置独立持久化，凭据仅复用供应商配置。
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Weak};
use std::time::{Duration, Instant};

use base64::{Engine, engine::general_purpose::STANDARD};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio::sync::Mutex;

use crate::settings::{BackendSettings, RelayProfile};

const MAX_ROLE_MEDIA: usize = 20 * 1024 * 1024;
const MAX_SMALL_MEDIA: usize = 8 * 1024 * 1024;
const MAX_TOTAL_MEDIA: usize = 256 * 1024 * 1024;
const MAX_ITEMS: usize = 200;
const MAX_DOC: usize = 512 * 1024;
const CURRENT: &str = "current-provider";
const CODEX: &str = "codex";
static FETCH_CACHE: LazyLock<Mutex<BTreeMap<String, (Instant, Value)>>> =
    LazyLock::new(|| Mutex::new(BTreeMap::new()));
static WRITE_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));
static ACCOUNT_GATES: LazyLock<std::sync::Mutex<BTreeMap<String, Weak<Mutex<()>>>>> =
    LazyLock::new(|| std::sync::Mutex::new(BTreeMap::new()));

fn account_gate(account: &str) -> Arc<Mutex<()>> {
    let mut gates = ACCOUNT_GATES
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(gate) = gates.get(account).and_then(Weak::upgrade) {
        return gate;
    }
    gates.retain(|_, gate| gate.strong_count() > 0);
    let gate = Arc::new(Mutex::new(()));
    gates.insert(account.into(), Arc::downgrade(&gate));
    gate
}

fn response(status: u16, body: Value) -> Value {
    json!({"status":status,"body":body})
}
fn failure(status: u16, message: &str) -> Value {
    response(status, json!({"ok":false,"error":message}))
}
fn error(message: &str) -> anyhow::Error {
    anyhow::anyhow!("{message}")
}
fn now() -> u64 {
    crate::whale::now_ms()
}
fn string<'a>(value: &'a Value, key: &str) -> &'a str {
    value[key].as_str().unwrap_or_default()
}
fn limited_name(value: &Value, key: &str, fallback: &str, limit: usize) -> String {
    let text = string(value, key).trim();
    if text.is_empty() {
        fallback.into()
    } else {
        text.chars().take(limit).collect()
    }
}
fn id_is_valid(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 80
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

fn open(path: &Path) -> anyhow::Result<Connection> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let connection = Connection::open(path)?;
    connection.busy_timeout(Duration::from_secs(2))?;
    connection.execute_batch("CREATE TABLE IF NOT EXISTS whale_full_documents(name TEXT PRIMARY KEY,json TEXT NOT NULL);
        CREATE TABLE IF NOT EXISTS whale_full_media(id TEXT PRIMARY KEY,kind TEXT NOT NULL,name TEXT NOT NULL,
        mime TEXT NOT NULL,format TEXT NOT NULL,bytes BLOB NOT NULL,created_at INTEGER NOT NULL,pinned_at INTEGER);")?;
    Ok(connection)
}

fn read_doc(path: &Path, name: &str, default: Value) -> anyhow::Result<Value> {
    if !path.exists() {
        return Ok(default);
    }
    let connection = Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let exists:bool=connection.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='whale_full_documents' AND type='table')",[],|r|r.get(0))?;
    if !exists {
        return Ok(default);
    }
    let text: Option<String> = connection
        .query_row(
            "SELECT json FROM whale_full_documents WHERE name=?1",
            [name],
            |r| r.get(0),
        )
        .optional()?;
    text.map(|s| serde_json::from_str(&s).map_err(|_| error("挂件设置损坏，已停止写入以保护记录")))
        .unwrap_or(Ok(default))
}

fn write_doc(path: &Path, name: &str, value: &Value) -> anyhow::Result<()> {
    let content = serde_json::to_string(value)?;
    if content.len() > MAX_DOC {
        return Err(error("设置内容过大，最多 512 KiB"));
    }
    let connection = open(path)?;
    connection.execute("INSERT INTO whale_full_documents(name,json) VALUES(?1,?2) ON CONFLICT(name) DO UPDATE SET json=excluded.json",params![name,content])?;
    Ok(())
}

fn merge(target: &mut Value, patch: &Value) {
    if let (Some(target), Some(patch)) = (target.as_object_mut(), patch.as_object()) {
        for (key, value) in patch {
            if value.is_object() {
                merge(
                    target.entry(key.clone()).or_insert_with(|| json!({})),
                    value,
                );
            } else {
                target.insert(key.clone(), value.clone());
            }
        }
    }
}

fn merge_nonempty(target: &mut Value, patch: &Value) {
    if let (Some(target), Some(patch)) = (target.as_object_mut(), patch.as_object()) {
        for (key, value) in patch {
            if value.is_null() || value == "" {
                continue;
            }
            if value.is_object() {
                merge_nonempty(
                    target.entry(key.clone()).or_insert_with(|| json!({})),
                    value,
                );
            } else {
                target.insert(key.clone(), value.clone());
            }
        }
    }
}

fn wait_lines(approval: bool) -> Value {
    json!([
    {"type":"session","size":10,"bold":true,"tpl":"[ {session} ]","len":5,"rgb":"champagne","color":""},
    {"type":"text","text":if approval {"正在等待老大授权"} else {"正在等待老大回答"},"size":7,"bold":true,"bgRgb":"","bg":"","rgb":"indigo","color":""}])
}

fn sound_events() -> Value {
    json!({"press":{"vol":1},"turnCost":{"vol":1,"volSet":false,"bubbleOn":true},
    "question":{"on":true,"soundOn":false,"sel":"frag:exp_orb","vol":1,"autoClose":true,"ttlSec":180,"bubbleOn":true,"lines":wait_lines(false)},
    "approval":{"on":true,"soundOn":false,"sel":"frag:exp_orb","vol":1,"autoClose":true,"ttlSec":180,"bubbleOn":true,"lines":wait_lines(true)}})
}

fn usage_defaults() -> Value {
    json!({"taskEnd":{"on":true,"sel":"frag:end_a"},"wait":{"charClose":false},"events":sound_events(),
    "alert":{"on":true,"below":5,"msg":"余额预警：当前余额已低于设定值 {below}","lines":[
        {"type":"text","text":"老大~你的 Codex 供应商余额","size":5,"bold":true},
        {"type":"text","text":"已经不足 {below} 啦~","size":5,"bold":true,"rgb":"rouge"},
        {"type":"image","imgId":"bimg_money1","size":6,"imgScale":0.4}],"autoClose":false,"ttlSec":6},
    "budget":{"on":true,"amount":10,"msg":"今日已用已达预算 {amount}","lines":[
        {"type":"text","text":"老大，今天花销已经超过 {amount} 啦，再花要变成穷光蛋啦...","size":6,"bold":true,"rgb":"rouge"}],"autoClose":false,"ttlSec":6},
    "turnCost":{"lines":[{"type":"text","text":"上一轮对话消耗:","size":8,"bold":true},
        {"type":"text","text":"{cost}","size":24,"bold":true,"color":"#e0433f"},
        {"type":"today","size":2,"tpl":"今日已用 {expense_ds}","bold":false,"color":"#ffffff","bgRgb":"indigo"}]},"models":{}})
}

fn size_defaults() -> Value {
    json!({"scale":1.5,"sound":true,"vol":0.9,"soundSet":"duck","usageMode":"ledger","peakMode":"default",
    "bubbleOn":true,"turnCostOn":true,"turnCostCloseMs":5000,"scrollGapOn":false,"scrollGapPx":17,"menuBtnHide":false,"codexStatsOn":true})
}

fn read_usage(path: &Path) -> anyhow::Result<Value> {
    let mut defaults = usage_defaults();
    merge(&mut defaults, &read_doc(path, "usage", json!({}))?);
    Ok(defaults)
}

fn presets() -> Vec<Value> {
    [
        ("ya1", "小黄鸭·按下"),
        ("ya2", "小黄鸭·松开"),
        ("d1", "音效1·按下"),
        ("d2", "音效1·松开"),
        ("exp_orb", "Minecraft·经验球"),
        ("end_a", "A"),
    ]
    .iter()
    .map(|(id, name)| json!({"id":id,"name":name,"preset":true}))
    .collect()
}
fn preset_groups() -> Vec<Value> {
    vec![
        json!({"id":"duck","name":"小黄鸭","press":"ya1","release":"ya2","preset":true,"pinned":false,"pinnedAt":null,"createdAt":0}),
        json!({"id":"fx1","name":"音效1","press":"d1","release":"d2","preset":true,"pinned":false,"pinnedAt":null,"createdAt":0}),
    ]
}

fn media_rows(path: &Path, kind: &str) -> anyhow::Result<Vec<Value>> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let connection = Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let exists: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='whale_full_media')",
        [],
        |r| r.get(0),
    )?;
    if !exists {
        return Ok(Vec::new());
    }
    let mut stmt=connection.prepare("SELECT id,name,mime,format,created_at,pinned_at FROM whale_full_media WHERE kind=?1 ORDER BY COALESCE(pinned_at,0) DESC,created_at DESC LIMIT 200")?;
    Ok(stmt.query_map([kind],|r| { let id:String=r.get(0)?; let pinned:Option<i64>=r.get(5)?;
        Ok(json!({"id":id,"name":r.get::<_,String>(1)?,"mimeType":r.get::<_,String>(2)?,"format":r.get::<_,String>(3)?,"createdAt":r.get::<_,i64>(4)?,"pinnedAt":pinned,"pinned":pinned.is_some(),"preset":false})) })?.collect::<rusqlite::Result<Vec<_>>>()?)
}

fn roles(path: &Path) -> anyhow::Result<Value> {
    let mut rows = media_rows(path, "role")?;
    for row in &mut rows {
        row["url"] = json!(format!(
            "/dsh-whale/role-image.png?id={}",
            string(row, "id")
        ));
    }
    let default = read_doc(path, "default-role", json!({"pinnedAt":1}))?;
    rows.push(json!({"id":"default","name":"小鲸鱼","url":"/dsh-whale/image.png","pinned":default["pinnedAt"].as_u64().unwrap_or(0)>0,"pinnedAt":default["pinnedAt"],"createdAt":0,"format":"png"}));
    rows.sort_by(|a, b| {
        b["pinnedAt"]
            .as_u64()
            .unwrap_or(0)
            .cmp(&a["pinnedAt"].as_u64().unwrap_or(0))
            .then_with(|| {
                b["createdAt"]
                    .as_u64()
                    .unwrap_or(0)
                    .cmp(&a["createdAt"].as_u64().unwrap_or(0))
            })
    });
    Ok(json!({"ok":true,"roles":rows}))
}

fn images(path: &Path) -> anyhow::Result<Value> {
    let mut rows = vec![
        json!({"id":"bimg_petpet","name":"petpet","format":"gif","url":"/dsh-whale/bubble-img.png?id=bimg_petpet","createdAt":null,"builtin":true}),
        json!({"id":"bimg_money1","name":"money1","format":"gif","url":"/dsh-whale/bubble-img.png?id=bimg_money1","createdAt":null,"builtin":true}),
    ];
    for mut row in media_rows(path, "image")? {
        row["url"] = json!(format!(
            "/dsh-whale/bubble-img.png?id={}",
            string(&row, "id")
        ));
        rows.push(row);
    }
    Ok(json!({"ok":true,"images":rows}))
}

fn audio(path: &Path) -> anyhow::Result<Value> {
    let mut fragments = presets();
    fragments.extend(media_rows(path, "audio")?);
    let custom = read_doc(path, "groups", json!([]))?;
    let mut pinned = Vec::new();
    let mut normal = Vec::new();
    for mut group in custom.as_array().cloned().unwrap_or_default() {
        group["preset"] = json!(false);
        group["pinned"] = json!(group["pinnedAt"].as_u64().unwrap_or(0) > 0);
        for (slot, fallback) in [("press", "ya1"), ("release", "ya2")] {
            let id = group[slot].as_str().unwrap_or(fallback);
            if !id.is_empty() && !fragments.iter().any(|v| v["id"] == id) {
                group[slot] = json!(fallback);
            }
        }
        if group["pinned"] == true {
            pinned.push(group);
        } else {
            normal.push(group);
        }
    }
    pinned.sort_by_key(|v| std::cmp::Reverse(v["pinnedAt"].as_u64().unwrap_or(0)));
    normal.sort_by_key(|v| std::cmp::Reverse(v["createdAt"].as_u64().unwrap_or(0)));
    let mut groups = pinned;
    groups.extend(preset_groups());
    groups.extend(normal);
    Ok(json!({"ok":true,"fragments":fragments,"groups":groups}))
}

fn decode_media(data: &str, kind: &str) -> anyhow::Result<(String, Vec<u8>)> {
    let (header, encoded) = data
        .split_once(',')
        .ok_or_else(|| error("无效的媒体数据"))?;
    let mime = header
        .strip_prefix("data:")
        .and_then(|v| v.strip_suffix(";base64"))
        .ok_or_else(|| error("只接受 base64 媒体上传"))?;
    let limit = if kind == "role" {
        MAX_ROLE_MEDIA
    } else {
        MAX_SMALL_MEDIA
    };
    let allowed = if kind == "audio" {
        mime == "audio/wav"
    } else {
        matches!(
            mime,
            "image/png" | "image/jpeg" | "image/gif" | "image/webp"
        )
    };
    if !allowed {
        return Err(error("媒体格式不支持"));
    }
    if encoded.len() > limit.div_ceil(3) * 4 {
        return Err(error(if kind == "role" {
            "角色图片超过 20 MiB"
        } else {
            "气泡图片或音频超过 8 MiB"
        }));
    }
    let bytes = STANDARD
        .decode(encoded)
        .map_err(|_| error("媒体 base64 格式无效"))?;
    if bytes.len() > limit {
        return Err(error(if kind == "role" {
            "角色图片超过 20 MiB"
        } else {
            "气泡图片或音频超过 8 MiB"
        }));
    }
    let valid = match mime {
        "image/png" => {
            bytes.len() >= 45
                && bytes.starts_with(b"\x89PNG\r\n\x1a\n")
                && &bytes[12..16] == b"IHDR"
                && bytes[bytes.len() - 12..bytes.len() - 8] == [0, 0, 0, 0]
                && &bytes[bytes.len() - 8..bytes.len() - 4] == b"IEND"
                && image_dimensions(
                    u32::from_be_bytes(bytes[16..20].try_into().unwrap()),
                    u32::from_be_bytes(bytes[20..24].try_into().unwrap()),
                )
        }
        "image/jpeg" => bytes.starts_with(&[255, 216, 255]) && bytes.ends_with(&[255, 217]),
        "image/gif" => {
            bytes.len() >= 14
                && (bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a"))
                && bytes.last() == Some(&b';')
                && image_dimensions(
                    u16::from_le_bytes(bytes[6..8].try_into().unwrap()) as u32,
                    u16::from_le_bytes(bytes[8..10].try_into().unwrap()) as u32,
                )
        }
        "image/webp" => bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP",
        "audio/wav" => valid_wav(&bytes),
        _ => false,
    };
    if !valid {
        return Err(error("文件内容与声明的媒体格式不一致"));
    }
    Ok((mime.into(), bytes))
}

fn image_dimensions(width: u32, height: u32) -> bool {
    width > 0
        && height > 0
        && width <= 16384
        && height <= 16384
        && u64::from(width) * u64::from(height) <= 64 * 1024 * 1024
}

fn valid_wav(bytes: &[u8]) -> bool {
    if bytes.len() < 44 || &bytes[..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return false;
    }
    let declared = u32::from_le_bytes(bytes[4..8].try_into().unwrap()) as usize + 8;
    if declared > bytes.len() || declared < 44 {
        return false;
    }
    let mut offset = 12usize;
    let mut fmt = false;
    let mut data = false;
    while offset + 8 <= declared {
        let len = u32::from_le_bytes(bytes[offset + 4..offset + 8].try_into().unwrap()) as usize;
        let start = offset + 8;
        let Some(end) = start.checked_add(len).filter(|end| *end <= declared) else {
            return false;
        };
        match &bytes[offset..offset + 4] {
            b"fmt " => {
                if len < 16 {
                    return false;
                }
                let encoding = u16::from_le_bytes(bytes[start..start + 2].try_into().unwrap());
                let channels = u16::from_le_bytes(bytes[start + 2..start + 4].try_into().unwrap());
                let rate = u32::from_le_bytes(bytes[start + 4..start + 8].try_into().unwrap());
                let bits = u16::from_le_bytes(bytes[start + 14..start + 16].try_into().unwrap());
                fmt = matches!(encoding, 1 | 3 | 65534)
                    && (1..=8).contains(&channels)
                    && (8000..=384000).contains(&rate)
                    && matches!(bits, 8 | 16 | 24 | 32 | 64);
            }
            b"data" => data = len > 0,
            _ => {}
        }
        offset = end + (len % 2);
    }
    fmt && data
}

fn upload(path: &Path, body: &Value, kind: &str) -> anyhow::Result<String> {
    let field = match kind {
        "role" => "image",
        "audio" => "audio",
        _ => "data",
    };
    let (mime, bytes) = decode_media(string(body, field), kind)?;
    let format = if mime == "image/gif" {
        "gif"
    } else if string(body, "format") == "apng" && mime == "image/png" {
        "apng"
    } else if kind == "audio" {
        "wav"
    } else {
        mime.strip_prefix("image/").unwrap_or("png")
    };
    let id = format!(
        "{}_{}",
        match kind {
            "role" => "role",
            "audio" => "audio",
            _ => "bimg",
        },
        uuid::Uuid::new_v4().simple()
    );
    let mut connection = open(path)?;
    let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let (count, total): (i64, i64) = tx.query_row(
        "SELECT count(*),COALESCE(sum(length(bytes)),0) FROM whale_full_media",
        [],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    if count >= MAX_ITEMS as i64 || total as usize + bytes.len() > MAX_TOTAL_MEDIA {
        return Err(error(
            "图库与音频库已达上限（200 项 / 256 MiB），请先删除不用的资源",
        ));
    }
    let name = limited_name(
        body,
        "name",
        if kind == "role" {
            "新角色"
        } else {
            "未命名"
        },
        40,
    );
    tx.execute("INSERT INTO whale_full_media(id,kind,name,mime,format,bytes,created_at) VALUES(?1,?2,?3,?4,?5,?6,?7)",params![id,kind,name,mime,format,bytes,now()])?;
    tx.commit()?;
    Ok(id)
}

fn media(path: &Path, id: &str, kind: &str) -> anyhow::Result<Value> {
    if kind == "audio" && presets().iter().any(|v| v["id"] == id) {
        return Ok(
            json!({"status":200,"body":null,"builtinFragmentId":id,"mimeType":if matches!(id,"exp_orb"|"end_a") {"audio/wav"} else {"audio/mpeg"}}),
        );
    }
    if !id_is_valid(id) || !path.exists() {
        return Ok(failure(404, "未找到媒体资源"));
    }
    let connection = open(path)?;
    let found: Option<(String, Vec<u8>)> = connection
        .query_row(
            "SELECT mime,bytes FROM whale_full_media WHERE id=?1 AND kind=?2",
            params![id, kind],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    Ok(match found {
        Some((mime, bytes)) => {
            json!({"status":200,"body":null,"mimeType":mime,"data":STANDARD.encode(bytes)})
        }
        None => failure(404, "未找到媒体资源"),
    })
}

fn remove_media(path: &Path, id: &str, kind: &str) -> anyhow::Result<()> {
    if !id_is_valid(id)
        || matches!(
            id,
            "default"
                | "bimg_petpet"
                | "bimg_money1"
                | "ya1"
                | "ya2"
                | "d1"
                | "d2"
                | "exp_orb"
                | "end_a"
        )
    {
        return Err(error("内置资源不能删除"));
    }
    let connection = open(path)?;
    if connection.execute(
        "DELETE FROM whale_full_media WHERE id=?1 AND kind=?2",
        params![id, kind],
    )? == 0
    {
        return Err(error("未找到媒体资源"));
    }
    Ok(())
}

fn media_operation(
    path: &Path,
    route: &str,
    method: &str,
    query: &Value,
    body: &Value,
) -> anyhow::Result<Value> {
    let writing = method != "GET";
    let result = match route {
        "/dsh-whale/roles.json" => {
            if writing {
                upload(path, body, "role")?;
            }
            roles(path)?
        }
        "/dsh-whale/role-pin.json" => {
            if !writing {
                return Err(error("请使用 POST"));
            }
            let id = string(body, "id");
            if id == "default" {
                write_doc(
                    path,
                    "default-role",
                    &json!({"pinnedAt":if body["pinned"]==true {Some(now())} else {None}}),
                )?;
                return Ok(response(200, roles(path)?));
            }
            if !id_is_valid(id) {
                return Err(error("角色 ID 无效"));
            }
            let connection = open(path)?;
            let pinned = if body["pinned"] == true {
                Some(now())
            } else {
                None
            };
            if connection.execute(
                "UPDATE whale_full_media SET pinned_at=?2 WHERE id=?1 AND kind='role'",
                params![id, pinned],
            )? == 0
            {
                return Err(error("未找到角色"));
            }
            roles(path)?
        }
        "/dsh-whale/role-delete.json" => {
            if !writing {
                return Err(error("请使用 POST"));
            }
            remove_media(path, string(body, "id"), "role")?;
            roles(path)?
        }
        "/dsh-whale/role-image.png" => return media(path, string(query, "id"), "role"),
        "/dsh-whale/bubble-imgs.json" => images(path)?,
        "/dsh-whale/bubble-img-upload.json" => {
            if !writing {
                return Err(error("请使用 POST"));
            }
            match string(body, "action") {
                "upload" => {
                    upload(path, body, "image")?;
                }
                "delete" => remove_media(path, string(body, "id"), "image")?,
                _ => return Err(error("未知图库操作")),
            }
            images(path)?
        }
        "/dsh-whale/bubble-img.png" => return media(path, string(query, "id"), "image"),
        "/dsh-whale/audio-fragment.wav" => return media(path, string(query, "id"), "audio"),
        "/dsh-whale/audio.json" => {
            let mut uploaded = None;
            if writing {
                match string(body, "action") {
                    "upload-fragment" => uploaded = Some(upload(path, body, "audio")?),
                    "delete-fragment" => remove_media(path, string(body, "id"), "audio")?,
                    "save-group" | "delete-group" | "pin-group" => {
                        let mut groups = read_doc(path, "groups", json!([]))?
                            .as_array()
                            .cloned()
                            .unwrap_or_default();
                        let id = string(body, "id");
                        if matches!(id, "duck" | "fx1") {
                            return Err(error("内置音效组不能修改或删除"));
                        }
                        match string(body, "action") {
                            "delete-group" => groups.retain(|v| string(v, "id") != id),
                            "pin-group" => {
                                let g = groups
                                    .iter_mut()
                                    .find(|v| string(v, "id") == id)
                                    .ok_or_else(|| error("未找到音效组"))?;
                                g["pinnedAt"] = if body["pinned"] == true {
                                    json!(now())
                                } else {
                                    Value::Null
                                };
                            }
                            _ => {
                                let all = audio(path)?;
                                let valid = |slot: &str| {
                                    let fid = string(body, slot);
                                    fid.is_empty()
                                        || all["fragments"].as_array().is_some_and(|xs| {
                                            xs.iter().any(|v| string(v, "id") == fid)
                                        })
                                };
                                if !valid("press") || !valid("release") {
                                    return Err(error("音效片段不存在，请重新选择"));
                                }
                                let gid = if id.is_empty() {
                                    format!("group_{}", uuid::Uuid::new_v4().simple())
                                } else {
                                    id.into()
                                };
                                if !id_is_valid(&gid) {
                                    return Err(error("音效组 ID 无效"));
                                }
                                let position = groups.iter().position(|v| string(v, "id") == gid);
                                let previous = position
                                    .map(|i| groups[i].clone())
                                    .unwrap_or(json!({"createdAt":now(),"pinnedAt":null}));
                                let group = json!({"id":gid,"name":limited_name(body,"name","未命名音效组",20),"press":string(body,"press"),"release":string(body,"release"),"createdAt":previous["createdAt"],"pinnedAt":previous["pinnedAt"]});
                                if let Some(i) = position {
                                    groups[i] = group;
                                } else {
                                    if groups.len() >= MAX_ITEMS {
                                        return Err(error("音效组数量达到上限"));
                                    }
                                    groups.push(group);
                                }
                            }
                        }
                        write_doc(path, "groups", &json!(groups))?;
                    }
                    _ => return Err(error("未知音频操作")),
                }
            }
            let mut value = audio(path)?;
            if let Some(id) = uploaded {
                value["id"] = json!(id);
            }
            value
        }
        "/dsh-whale/sound/press.mp3" | "/dsh-whale/sound/release.mp3" => {
            let slot = if route.ends_with("press.mp3") {
                "press"
            } else {
                "release"
            };
            let groups = audio(path)?;
            let group = groups["groups"]
                .as_array()
                .and_then(|xs| xs.iter().find(|v| string(v, "id") == string(query, "set")))
                .ok_or_else(|| error("未找到音效组"))?;
            let fid = string(group, slot);
            if fid.is_empty() {
                return Ok(json!({"status":204,"body":null}));
            }
            return media(path, fid, "audio");
        }
        _ => return Err(error("未知媒体接口")),
    };
    Ok(response(200, result))
}

fn settings_operation(
    path: &Path,
    route: &str,
    method: &str,
    body: &Value,
) -> anyhow::Result<Value> {
    let writing = method != "GET";
    Ok(response(
        200,
        match route {
            "/dsh-whale/size.json" => {
                let stored = read_doc(path, "size", Value::Null)?;
                let mut config = size_defaults();
                merge(&mut config, &stored);
                if writing {
                    for key in ["scale", "vol", "turnCostCloseMs", "scrollGapPx"] {
                        if let Some(v) = body.get(key) {
                            if !v.is_number() || !v.as_f64().is_some_and(f64::is_finite) {
                                return Err(error("尺寸、音量与时间必须是有限数字"));
                            }
                        }
                    }
                    let keys = [
                        "scale",
                        "sound",
                        "vol",
                        "soundSet",
                        "peakMode",
                        "bubbleOn",
                        "turnCostOn",
                        "turnCostCloseMs",
                        "scrollGapOn",
                        "scrollGapPx",
                        "menuBtnHide",
                        "codexStatsOn",
                    ];
                    for key in keys {
                        if let Some(v) = body.get(key) {
                            config[key] = v.clone();
                        }
                    }
                    config["scale"] =
                        json!(config["scale"].as_f64().unwrap_or(1.5).clamp(0.6, 2.5));
                    config["vol"] = json!(config["vol"].as_f64().unwrap_or(0.9).clamp(0.0, 1.0));
                    config["turnCostCloseMs"] = json!(
                        config["turnCostCloseMs"]
                            .as_f64()
                            .unwrap_or(5000.0)
                            .clamp(0.0, 3_600_000.0)
                    );
                    config["scrollGapPx"] = json!(
                        config["scrollGapPx"]
                            .as_f64()
                            .unwrap_or(17.0)
                            .clamp(0.0, 256.0)
                    );
                    write_doc(path, "size", &config)?;
                }
                config["ok"] = json!(true);
                config["hasSavedConfig"] = json!(writing || !stored.is_null());
                config
            }
            "/dsh-whale/bubble.json" => {
                let config = if writing {
                    if !body["items"].is_array()
                        || !body["lib"].is_array()
                        || body["items"].as_array().unwrap().len() > 200
                        || body["lib"].as_array().unwrap().len() > 200
                    {
                        return Err(error("泡泡配置无效或超过 200 个条目"));
                    }
                    let config = json!({"v":1,"items":body["items"],"lib":body["lib"],"tapAdvance":body["tapAdvance"]==true});
                    write_doc(path, "bubble", &config)?;
                    config
                } else {
                    read_doc(path, "bubble", Value::Null)?
                };
                json!({"ok":true,"config":config})
            }
            "/dsh-whale/usage-settings.json" => {
                let mut config = read_usage(path)?;
                if writing {
                    if !body.is_object() {
                        return Err(error("提醒配置无效"));
                    }
                    for key in ["taskEnd", "alert", "budget", "turnCost", "wait", "events"] {
                        if let Some(v) = body.get(key).filter(|v| v.is_object()) {
                            merge(&mut config[key], v);
                        }
                    }
                    if body["resetEvents"] == true {
                        config["events"] = sound_events();
                        config["wait"] = json!({"charClose":false});
                        config["taskEnd"] = json!({"on":true,"sel":"frag:end_a"});
                    }
                    if let Some(m) = body.get("modelSettings").filter(|v| v.is_object()) {
                        let id = string(m, "id");
                        if !id_is_valid(id) {
                            return Err(error("模型 ID 无效"));
                        }
                        for key in ["alert", "budget", "quota"] {
                            if let Some(v) = m.get(key).filter(|v| v.is_object()) {
                                if config["models"][id].is_null() {
                                    config["models"][id] = json!({});
                                }
                                if config["models"][id][key].is_null() {
                                    config["models"][id][key] = json!({});
                                }
                                merge(&mut config["models"][id][key], v);
                            }
                        }
                    }
                    if let Some(events) = config["events"]["turnCost"].as_object_mut() {
                        events.remove("autoClose");
                        events.remove("ttlSec");
                    }
                    write_doc(path, "usage", &config)?;
                }
                json!({"ok":true,"settings":config})
            }
            _ => return Err(error("未知设置接口")),
        },
    ))
}

pub fn history_query(path: &Path) -> Value {
    let models = read_doc(path, "models", json!([])).unwrap_or(json!([]));
    let mut prices = Vec::new();
    for model in models.as_array().into_iter().flatten() {
        let price = &model["price"];
        if price.is_object() {
            for id in model["matchIds"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
            {
                let currency = if string(price, "cur").is_empty() {
                    "CNY"
                } else {
                    string(price, "cur")
                };
                let multiplier = if currency == "CNY" {
                    Some(1.0)
                } else if currency == "USD" {
                    number(&price["rate"]).filter(|v| *v > 0.0 && *v <= 1000.0)
                } else {
                    None
                };
                if let Some(multiplier) = multiplier.filter(|_| {
                    ["miss", "hit", "out"]
                        .iter()
                        .all(|k| number(&price[*k]).is_some_and(|v| v >= 0.0))
                }) {
                    prices.push(json!({"model":id,"input":number(&price["miss"]).unwrap()*multiplier,"cachedInput":number(&price["hit"]).unwrap()*multiplier,"output":number(&price["out"]).unwrap()*multiplier,"currency":"CNY"}));
                }
            }
        }
    }
    json!({"prices":prices,"period":"all","page":1,"pageSize":500})
}

pub fn history_enabled(path: &Path) -> bool {
    read_doc(path, "size", Value::Null)
        .map(|v| v["codexStatsOn"] != false)
        .unwrap_or(false)
}

fn template(id: &str, name: &str, currency: &str, url: &str, mapping: Value) -> Value {
    json!({"id":id,"name":name,"currency":currency,"keyRef":"","builtin":false,"needsBaseUrl":false,"hasBalance":!url.is_empty(),"probeUrl":"","kind":"balance",
        "balance":{"url":url,"auth":"Bearer {key}","json":mapping},"quota":null,"matchIds":[],"noBalanceApi":url.is_empty(),"apiNote":if url.is_empty() {"没有公开余额接口；可使用会话 token 统计与自定义计价"} else {""},"sortKey":name.to_lowercase()})
}

fn templates() -> Vec<Value> {
    // 原版 34 项模板逐字段保留。地址来自固定版本源码，凭据目的地仍逐项校验。
    let source: Value = serde_json::from_str(include_str!(
        "../../../assets/inject/upstream/whale-widget/provider-templates.json"
    ))
    .expect("checked-in provider templates must be valid JSON");
    source["templates"]
        .as_object()
        .into_iter()
        .flatten()
        .map(|(id, original)| {
            let mut value = original.clone();
            value["id"] = json!(id);
            value["defaultKeyRef"] = original["keyRef"].clone();
            value["keyRef"] = json!("");
            value["hasBalance"] = json!(!string(&original["balance"], "url").is_empty());
            value["kind"] = json!(if string(original, "kind").is_empty() {
                "balance"
            } else {
                string(original, "kind")
            });
            value["sortKey"] = json!(string(original, "name").to_lowercase());
            value
        })
        .collect()
}

fn infer_template(profile: &RelayProfile) -> &'static str {
    let base = crate::relay_config::relay_profile_base_url(profile);
    let Ok(url) = reqwest::Url::parse(&base) else {
        return "custom";
    };
    match url.host_str().unwrap_or_default() {
        "api.deepseek.com" => "deepseek",
        "openrouter.ai" => "openrouter",
        "api.moonshot.cn" => "moonshot",
        "api.moonshot.ai" => "moonshot_intl",
        "api.stepfun.com" => "stepfun",
        "api.novita.ai" => "novita",
        "open.bigmodel.cn" => "zhipu_glm_coding",
        "api.z.ai" => "zhipu_glm_coding_intl",
        "api.kimi.com" => "kimi_coding",
        "api.minimaxi.com" if base.contains("coding") => "minimax_coding",
        "api.minimax.io" if base.contains("coding") => "minimax_coding_intl",
        "opencode.ai" if base.contains("/go/") => "opencode_go",
        _ => "custom",
    }
}

fn monitor_models(settings: &BackendSettings, path: &Path) -> anyhow::Result<Vec<Value>> {
    let profile = settings.active_relay_profile();
    let mut main = json!({"id":CURRENT,"name":"当前供应商","provider":infer_template(&profile),"currency":"USD","keyRef":profile.id,"builtin":true,"matchIds":[]});
    if let Some(t) = templates().iter().find(|t| t["id"] == main["provider"]) {
        main["currency"] = t["currency"].clone();
    }
    main["matchIds"] = json!(
        profile
            .model_list
            .lines()
            .filter_map(|line| line.split('[').next())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
    );
    if settings.codex_app_whale_balance_protocol == "custom" {
        main["provider"] = json!("custom");
        main["currency"] = json!(settings.codex_app_whale_balance_currency);
        main["balance"] = json!({"url":settings.codex_app_whale_balance_path,"auth":"Bearer {key}","json":{"remaining":settings.codex_app_whale_balance_field,"scale":settings.codex_app_whale_balance_scale}});
    }
    let mut out = vec![
        main,
        json!({"id":CODEX,"name":"Codex 本机用量","provider":CODEX,"currency":"USD","keyRef":"","builtin":false,"matchIds":[]}),
    ];
    for model in read_doc(path, "models", json!([]))?
        .as_array()
        .into_iter()
        .flatten()
    {
        if string(model, "id") == CURRENT {
            if model["keyRef"] == profile.id {
                merge(&mut out[0], model);
            }
        } else if string(model, "id") == CODEX {
            merge(&mut out[1], model);
        } else {
            out.push(model.clone());
        }
    }
    Ok(out)
}

fn profile_for(settings: &BackendSettings, model: &Value) -> Option<RelayProfile> {
    if string(model, "id") == CURRENT && string(model, "keyRef").is_empty() {
        return Some(settings.active_relay_profile());
    }
    let reference = string(model, "keyRef");
    settings
        .relay_profiles
        .iter()
        .find(|p| p.id == reference)
        .cloned()
        .or_else(|| {
            let active = settings.active_relay_profile();
            (active.id == reference).then_some(active)
        })
}

fn value_at<'a>(value: &'a Value, path: &str) -> Option<&'a Value> {
    if path.len() > 256 {
        return None;
    }
    let flattened = path.replace('[', ".").replace(']', "");
    let mut selected = value;
    for key in flattened.split('.').filter(|s| !s.is_empty()) {
        selected = if selected.is_array() {
            selected.get(key.parse::<usize>().ok()?)?
        } else {
            selected.get(key)?
        };
    }
    Some(selected)
}
fn number(value: &Value) -> Option<f64> {
    value
        .as_f64()
        .or_else(|| value.as_str()?.parse().ok())
        .filter(|n| n.is_finite())
}
fn field_number(value: &Value, path: &str, scale: f64) -> Option<f64> {
    if path.is_empty() {
        return None;
    }
    let n = number(value_at(value, path)?)? * scale;
    (n.is_finite() && n.abs() <= 100_000_000.0).then_some(n)
}

#[cfg(test)]
fn endpoint(profile: &RelayProfile, raw: &str) -> anyhow::Result<reqwest::Url> {
    endpoint_allowed(profile, raw, &Value::Null)
}

fn endpoint_allowed(
    profile: &RelayProfile,
    raw: &str,
    approved: &Value,
) -> anyhow::Result<reqwest::Url> {
    let base = reqwest::Url::parse(&crate::relay_config::relay_profile_base_url(profile))
        .map_err(|_| error("供应商地址无效"))?;
    let base_text = base.as_str().trim_end_matches('/').trim_end_matches("/v1");
    let expanded = raw.replace("{base}", base_text);
    let url = reqwest::Url::parse(&expanded)
        .or_else(|_| base.join(&format!("/{}", expanded.trim_start_matches('/'))))
        .map_err(|_| error("接口地址无效"))?;
    let local = matches!(
        url.host_str(),
        Some("127.0.0.1" | "localhost" | "[::1]" | "::1")
    );
    let explicitly_allowed = approved.as_array().is_some_and(|xs| {
        xs.iter()
            .any(|v| v.as_str() == Some(url.origin().ascii_serialization().as_str()))
    });
    if url.origin() != base.origin() && !explicitly_allowed
        || !(url.scheme() == "https" || url.scheme() == "http" && local)
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
        || expanded.contains("{key}")
        || expanded.contains("{api_key}")
    {
        return Err(error(
            "余额与额度接口必须与所选供应商同源，使用 HTTPS（本机回环地址可用 HTTP），且网址不能包含凭据",
        ));
    }
    if url.host_str().is_some_and(|h| {
        matches!(
            h,
            "metadata.google.internal" | "metadata.goog" | "100.100.100.200"
        ) || h.starts_with("169.254.")
            || h.starts_with("fe80:")
    }) {
        return Err(error("不能访问云元数据或 link-local 地址"));
    }
    Ok(url)
}

fn fill_strings(value: &mut Value, key: &str, base: &str, params: &Value) {
    match value {
        Value::String(text) => {
            *text = text.replace("{key}", key).replace("{base}", base);
            if let Some(params) = params.as_object() {
                for (name, value) in params {
                    if !value.is_object() && !value.is_array() && !value.is_null() {
                        let s = value
                            .as_str()
                            .map(str::to_owned)
                            .unwrap_or_else(|| value.to_string());
                        *text = text.replace(&format!("{{{name}}}"), &s);
                    }
                }
            }
        }
        Value::Array(xs) => {
            for v in xs {
                fill_strings(v, key, base, params);
            }
        }
        Value::Object(xs) => {
            for v in xs.values_mut() {
                fill_strings(v, key, base, params);
            }
        }
        _ => {}
    }
}

async fn fetch(
    profile: &RelayProfile,
    descriptor: &Value,
    params_value: &Value,
    approved: &Value,
) -> anyhow::Result<Value> {
    let query_key = descriptor["queryKey"] == true;
    let raw = string(descriptor, "url");
    let mut url = endpoint_allowed(
        profile,
        if query_key {
            raw.replace("{key}", "placeholder")
        } else {
            raw.to_string()
        }
        .as_str(),
        approved,
    )?;
    let key = crate::relay_config::relay_profile_api_key(profile);
    let auth = descriptor
        .get("auth")
        .and_then(Value::as_str)
        .unwrap_or("Bearer {key}");
    if (!auth.is_empty() || query_key) && (key.is_empty() || profile.uses_no_auth()) {
        return Err(error("所选供应商未配置可用的 API 密钥"));
    }
    if query_key {
        let fields = url
            .query_pairs()
            .map(|(name, value)| {
                (
                    name.to_string(),
                    if name == "key" && value == "placeholder" {
                        key.clone()
                    } else {
                        value.to_string()
                    },
                )
            })
            .collect::<Vec<_>>();
        url.query_pairs_mut().clear().extend_pairs(fields);
    }
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(8))
        .connect_timeout(Duration::from_secs(4))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| error("查询暂不可用"))?;
    let method = match string(descriptor, "method") {
        "" | "GET" => reqwest::Method::GET,
        "POST" => reqwest::Method::POST,
        "PUT" => reqwest::Method::PUT,
        "PATCH" => reqwest::Method::PATCH,
        _ => return Err(error("监测接口只支持 GET、POST、PUT 或 PATCH")),
    };
    let mut request = client.request(method.clone(), url);
    match auth {
        "" => {}
        "Bearer {key}" => request = request.bearer_auth(&key),
        "{key}" => request = request.header(reqwest::header::AUTHORIZATION, &key),
        _ => return Err(error("鉴权格式只支持 Bearer {key} 或 {key}")),
    }
    if method != reqwest::Method::GET {
        let mut body: Value = serde_json::from_str(string(descriptor, "body"))
            .map_err(|_| error("POST 请求体必须是有效 JSON"))?;
        fill_strings(
            &mut body,
            &key,
            &crate::relay_config::relay_profile_base_url(profile),
            params_value,
        );
        request = request
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(body.to_string());
    }
    let mut response = request
        .send()
        .await
        .map_err(|_| error("查询失败，请检查网络连接"))?;
    if !response.status().is_success() {
        return Err(error(match response.status().as_u16() {
            401 | 403 => "查询鉴权失败，请检查供应商密钥",
            429 => "查询频率受限，请稍后重试",
            _ => "供应商暂时无法提供监测数据",
        }));
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| error("响应读取失败"))? {
        if bytes.len() + chunk.len() > 64 * 1024 {
            return Err(error("监测响应超过 64 KiB"));
        }
        bytes.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&bytes).map_err(|_| error("监测响应不是有效 JSON"))
}

fn reset_at(value: Option<&Value>) -> Option<i64> {
    let value = value?;
    if let Some(n) = number(value) {
        return Some(if n > 1e12 {
            (n / 1000.0) as i64
        } else {
            n as i64
        })
        .filter(|v| *v > 0);
    }
    let text = value.as_str()?;
    Connection::open_in_memory()
        .ok()?
        .query_row("SELECT unixepoch(?1)", [text], |r| {
            r.get::<_, Option<i64>>(0)
        })
        .ok()
        .flatten()
}

fn quota_payload(data: &Value, mapping: &Value) -> anyhow::Result<Value> {
    let mut windows = Vec::new();
    if let Some(descriptors) = mapping["windows"].as_array() {
        for w in descriptors.iter().take(6) {
            let selected = if w["pick"].is_object() {
                let pick = &w["pick"];
                value_at(data, string(pick, "list"))
                    .and_then(Value::as_array)
                    .and_then(|xs| {
                        xs.iter().find(|v| {
                            let types = pick["types"].as_array();
                            let wanted = string(pick, "type");
                            (types.is_none_or(|ts| ts.iter().any(|t| t == &v["type"]))
                                && (wanted.is_empty() || v["type"] == wanted))
                                && (pick.get("unit").is_none() || pick["unit"] == v["unit"])
                                && (pick.get("number").is_none() || pick["number"] == v["number"])
                        })
                    })
            } else {
                Some(data)
            };
            if let Some(selected) = selected {
                let percent = field_number(selected, string(w, "percent"), 1.0)
                    .filter(|n| (0.0..=100.0).contains(n));
                let reset = reset_at(value_at(selected, string(w, "resetAt")));
                if percent.is_some() || reset.is_some() {
                    windows.push(json!({"key":w["key"],"label":w["label"],"percent":percent,"usedPct":percent,"resetAt":reset}));
                }
            }
        }
    } else {
        let percent = field_number(data, string(mapping, "percent"), 1.0)
            .or_else(|| field_number(data, string(mapping, "remainPct"), 1.0).map(|v| 100.0 - v))
            .or_else(|| {
                let remain = field_number(data, string(mapping, "remain"), 1.0)?;
                let total = field_number(data, string(mapping, "total"), 1.0)?;
                (total > 0.0).then_some((1.0 - remain / total) * 100.0)
            })
            .filter(|n| (0.0..=100.0).contains(n));
        let reset_path = if string(mapping, "resetAtMs").is_empty() {
            string(mapping, "resetAt")
        } else {
            string(mapping, "resetAtMs")
        };
        let reset = reset_at(value_at(data, reset_path));
        if percent.is_some() || reset.is_some() {
            windows.push(json!({"key":"rolling","label":"当前窗口","percent":percent,"usedPct":percent,"resetAt":reset}));
        }
        if let Some(percent) = field_number(data, string(mapping, "weeklyRemainPct"), 1.0)
            .map(|v| 100.0 - v)
            .filter(|n| (0.0..=100.0).contains(n))
        {
            windows.push(json!({"key":"weekly","label":"周","percent":percent,"usedPct":percent,"resetAt":null}));
        }
    }
    if windows.is_empty() {
        return Err(error("额度响应中未找到配置的有效窗口字段"));
    }
    let primary = windows[0].clone();
    let used = number(&primary["usedPct"]);
    let weekly = windows
        .iter()
        .find(|w| w["key"] == "weekly")
        .map(|w| w["usedPct"].clone())
        .unwrap_or(Value::Null);
    Ok(
        json!({"ok":true,"percent":primary["percent"],"usedPct":used,"remainPct":used.map(|v|100.0-v),"weeklyUsedPct":weekly,"resetAt":primary["resetAt"],"windows":windows,"level":value_at(data,string(mapping,"level")).filter(|v|v.is_string()||v.is_number()),"observedAt":now()}),
    )
}

async fn model_observation(
    settings: &BackendSettings,
    path: &Path,
    model: &Value,
    force: bool,
) -> Value {
    if string(model, "provider") == CODEX {
        return json!({"balance":null,"hasBalanceApi":false,"balanceMode":"events","error":null});
    }
    let Some(profile) = profile_for(settings, model) else {
        return json!({"balance":null,"error":"所选供应商配置不存在，请重新选择配置 ID","hasKey":false});
    };
    let tpl = templates()
        .into_iter()
        .find(|v| string(v, "id") == string(model, "provider"))
        .unwrap_or_else(|| template("custom", "自定义 HTTP", "USD", "", json!({})));
    let mut balance = tpl["balance"].clone();
    if balance.is_null() {
        balance = json!({});
    }
    merge_nonempty(&mut balance, &model["balance"]);
    let mut quota = tpl["quota"].clone();
    if quota.is_null() && model["quotaDesc"].is_object() {
        quota = json!({});
    }
    merge_nonempty(&mut quota, &model["quotaDesc"]);
    let current_disabled = string(model, "id") == CURRENT
        && (!settings.relay_profiles_enabled
            || profile.relay_mode == crate::settings::RelayMode::Aggregate);
    let has_balance = !string(&balance, "url").is_empty()
        && !current_disabled
        && !(string(model, "id") == CURRENT && settings.codex_app_whale_balance_protocol == "off");
    let has_quota = !string(&quota, "url").is_empty() && !current_disabled;
    let key = crate::relay_config::relay_profile_api_key(&profile);
    let fingerprint = format!(
        "{:x}",
        Sha256::digest(
            json!([profile.id, key, model, balance, quota])
                .to_string()
                .as_bytes()
        )
    );
    let account = format!(
        "{:x}",
        Sha256::digest(
            json!([
                crate::relay_config::relay_profile_base_url(&profile),
                key,
                balance,
                model["currency"]
            ])
            .to_string()
            .as_bytes()
        )
    );
    let cache_key = format!("{}:{fingerprint}", path.to_string_lossy());
    // 同账本账户的查询按顺序完成，避免两窗口的旧网络样本晚到产生虚假的消费。
    let gate = account_gate(&format!("{}:{account}", path.to_string_lossy()));
    let _guard = gate.lock().await;
    let previous = FETCH_CACHE.lock().await.get(&cache_key).cloned();
    let current_day = crate::whale::local_day().ok();
    if !force {
        if let Some((at, value)) = &previous {
            let day_matches =
                !has_balance || value["accounting"]["day"].as_str() == current_day.as_deref();
            if at.elapsed() < Duration::from_secs(25) && day_matches {
                return value.clone();
            }
        }
    }
    let mut out = json!({"accountId":account,"hasKey":!key.is_empty()&&!profile.uses_no_auth(),"baseUrl":crate::relay_config::relay_profile_base_url(&profile),
        "balanceDesc":balance,"hasBalanceApi":has_balance,"balanceMode":if has_balance {"api"} else {"events"},"planSupport":has_quota,"balance":null,"error":null,"stale":false,"canAdjustBalance":has_balance,"apiNote":tpl["apiNote"]});
    out["probeUrl"] = tpl["probeUrl"].clone();
    out["quotaDesc"] = quota.clone();
    out["noBalanceApi"] = json!(!has_balance);
    out["needsHostConfirm"] = json!([&balance, &balance["usage"], &quota].iter().any(|desc| {
        !string(desc, "url").is_empty()
            && endpoint_allowed(&profile, string(desc, "url"), &model["approvedOrigins"]).is_err()
    }));
    if has_balance {
        let result = async {
            let data = fetch(
                &profile,
                &balance,
                &model["params"],
                &model["approvedOrigins"],
            )
            .await?;
            let mapping = &balance["json"];
            let scale = if mapping["scale"].is_null() {
                1.0
            } else {
                number(&mapping["scale"])
                    .filter(|v| *v > 0.0)
                    .ok_or_else(|| error("余额倍率必须是有限正数"))?
            };
            let total = field_number(&data, string(mapping, "total"), scale);
            let mut used = if string(mapping, "used").is_empty() {
                0.0
            } else {
                field_number(&data, string(mapping, "used"), scale)
                    .ok_or_else(|| error("响应中未找到配置的已用金额字段"))?
            };
            if !string(&balance["usage"], "url").is_empty() {
                let d = fetch(
                    &profile,
                    &balance["usage"],
                    &model["params"],
                    &model["approvedOrigins"],
                )
                .await?;
                let m = &balance["usage"]["json"];
                let scale = if m["scale"].is_null() {
                    1.0
                } else {
                    number(&m["scale"])
                        .filter(|v| *v > 0.0)
                        .ok_or_else(|| error("用量倍率必须是有限正数"))?
                };
                used += field_number(&d, string(m, "used"), scale)
                    .ok_or_else(|| error("用量响应字段不存在"))?;
            }
            let remaining = field_number(&data, string(mapping, "remaining"), scale)
                .or_else(|| total.map(|v| v - used))
                .ok_or_else(|| error("响应中未找到配置的余额字段"))?;
            if !remaining.is_finite() || remaining.abs() > 100_000_000.0 {
                return Err(error("余额换算结果超出范围"));
            }
            let currency = string(model, "currency");
            let accounting = crate::whale::observe_snapshot(path, &account, currency, remaining)?;
            Ok::<_, anyhow::Error>((remaining, accounting, data))
        }
        .await;
        match result {
            Ok((remaining, accounting, data)) => {
                out["balance"] = json!(remaining);
                out["accounting"] = accounting.clone();
                out["todayUsage"] = accounting["amount"].clone();
                out["todayUsageCurrency"] = accounting["currency"].clone();
                out["usageSource"] = accounting["source"].clone();
                out["usageLabel"] = accounting["label"].clone();
                if string(model, "provider") == "deepseek" {
                    out["bonusBalance"] = value_at(&data, "balance_infos[0].granted_balance")
                        .and_then(number)
                        .map(|v| json!(v))
                        .unwrap_or(Value::Null);
                    out["rechargeBalance"] = value_at(&data, "balance_infos[0].topped_up_balance")
                        .and_then(number)
                        .map(|v| json!(v))
                        .unwrap_or(Value::Null);
                }
            }
            Err(err) => {
                out["error"] = json!(err.to_string());
                if let Some((_, value)) = &previous {
                    if value["balance"].is_number() {
                        for k in ["balance", "bonusBalance", "rechargeBalance"] {
                            out[k] = value[k].clone();
                        }
                        if value["accounting"]["day"].as_str() == current_day.as_deref() {
                            for k in [
                                "accounting",
                                "todayUsage",
                                "todayUsageCurrency",
                                "usageSource",
                                "usageLabel",
                            ] {
                                out[k] = value[k].clone();
                            }
                        }
                        out["stale"] = json!(true);
                    }
                }
            }
        }
    }
    if has_quota {
        out["plan"] = match fetch(
            &profile,
            &quota,
            &model["params"],
            &model["approvedOrigins"],
        )
        .await
        .and_then(|data| quota_payload(&data, &quota["json"]))
        {
            Ok(v) => v,
            Err(e) => json!({"ok":false,"error":e.to_string()}),
        };
    }
    let mut cache = FETCH_CACHE.lock().await;
    if cache.len() >= 128 && !cache.contains_key(&cache_key) {
        if let Some(first) = cache
            .iter()
            .min_by_key(|(_, v)| v.0)
            .map(|(k, _)| k.clone())
        {
            cache.remove(&first);
        }
    }
    cache.insert(cache_key, (Instant::now(), out.clone()));
    out
}

fn quota_auto_used(quota: &Value, history: &Value) -> Option<u64> {
    if quota["unit"] == "money" || quota["mode"] == "manual" {
        return number(&quota["used"])
            .filter(|n| *n >= 0.0)
            .map(|v| v as u64);
    }
    let key = match string(quota, "reset") {
        "daily" => "todayTokens",
        "monthly" => "monthTokens",
        _ => "totalTokens",
    };
    let machine = history.get("machineSummary").unwrap_or(history);
    let total = machine[key].as_u64()?;
    if key == "totalTokens" {
        Some(
            total
                .saturating_sub(quota["baseAt"].as_u64().unwrap_or(0))
                .saturating_add(quota["used"].as_u64().unwrap_or(0)),
        )
    } else {
        Some(total)
    }
}

fn model_matches(model: &Value, name: &str) -> bool {
    if model["provider"] == CODEX {
        return true;
    }
    let lower = name.to_lowercase();
    model["matchIds"].as_array().is_some_and(|xs| {
        xs.iter().filter_map(Value::as_str).any(|id| {
            !id.is_empty()
                && (id.eq_ignore_ascii_case(name)
                    || id.len() >= 3 && lower.contains(&id.to_lowercase()))
        })
    })
}

fn model_token_total(model: &Value, quota: &Value, history: &Value) -> Option<u64> {
    if quota["mode"] == "codex" || model["provider"] == CODEX {
        let mut q = quota.clone();
        q["baseAt"] = json!(0);
        q["used"] = json!(0);
        return quota_auto_used(&q, history);
    }
    let period = string(quota, "reset");
    let today = crate::whale::local_day().ok()?;
    let models = if matches!(period, "daily" | "monthly") {
        let rows = history["days"].as_array()?;
        let mut items = Vec::new();
        for day in rows {
            let date = string(day, "date");
            if period == "daily" && date != today
                || period == "monthly" && !date.starts_with(&today[..7])
            {
                continue;
            }
            items.extend(day["models"].as_array().cloned().unwrap_or_default());
        }
        items
    } else {
        history["models"].as_array()?.clone()
    };
    let mut total = 0u64;
    let mut matched = false;
    for row in models {
        if model_matches(model, string(&row, "model")) {
            matched = true;
            total = total.saturating_add(row["usage"]["totalTokens"].as_u64()?);
        }
    }
    if matched {
        Some(total)
    } else if history["complete"] == true {
        Some(0)
    } else {
        None
    }
}

fn model_quota_used(model: &Value, quota: &Value, history: &Value) -> Value {
    if quota["mode"] == "manual" || quota["unit"] == "money" {
        return quota["used"].clone();
    }
    model_token_total(model, quota, history)
        .map(|total| {
            let used = if matches!(string(quota, "reset"), "daily" | "monthly") {
                total
            } else {
                total
                    .saturating_sub(quota["baseAt"].as_u64().unwrap_or(0))
                    .saturating_add(quota["used"].as_u64().unwrap_or(0))
            };
            json!(used)
        })
        .unwrap_or(Value::Null)
}

fn today_estimate(model: &Value, history: &Value) -> Option<(f64, bool)> {
    let today = crate::whale::local_day().ok()?;
    let rows = history["days"]
        .as_array()?
        .iter()
        .find(|v| v["date"] == today)?["models"]
        .as_array()?;
    let mut amount = 0.0;
    let mut priced = false;
    let mut partial = history["complete"] != true;
    for row in rows
        .iter()
        .filter(|row| model_matches(model, string(row, "model")))
    {
        partial |= row["unpricedRecords"].as_u64().unwrap_or(0) > 0
            || row["unknownUsageRecords"].as_u64().unwrap_or(0) > 0;
        if let Some(cost) = row["estimatedCosts"]
            .as_array()
            .and_then(|xs| xs.iter().find(|v| v["currency"] == "CNY"))
        {
            if let Some(value) = number(&cost["amount"]) {
                amount += value;
                priced = true;
            }
        }
    }
    priced.then_some((amount, partial))
}

fn model_usage_patch(
    settings: &BackendSettings,
    path: &Path,
    body: &Value,
    history: &Value,
) -> anyhow::Result<()> {
    let id = string(body, "id");
    let model = monitor_models(settings, path)?
        .into_iter()
        .find(|v| string(v, "id") == id)
        .ok_or_else(|| error("监测模型不存在"))?;
    let mut patched = body.clone();
    if patched["quota"].is_object() {
        if patched["quota"]["resetBase"] == true {
            let mut quota = patched["quota"].clone();
            quota["reset"] = json!("none");
            let base = model_token_total(&model, &quota, history)
                .ok_or_else(|| error("本机用量尚未统计完成，请稍后重置基准"))?;
            patched["quota"]["baseAt"] = json!(base);
        }
        if let Some(quota) = patched["quota"].as_object_mut() {
            quota.remove("resetBase");
        }
        for key in ["total", "used", "baseAt"] {
            if !patched["quota"][key].is_null()
                && !number(&patched["quota"][key])
                    .is_some_and(|v| v >= 0.0 && v <= 9_007_199_254_740_991.0)
            {
                return Err(error("额度总量、已用与基准须为非负有限数"));
            }
        }
    }
    let mut patch = json!({"modelSettings":patched});
    if id == CURRENT {
        for field in ["alert", "budget"] {
            if let Some(value) = body.get(field).filter(|v| v.is_object()) {
                patch[field] = value.clone();
            }
        }
    }
    settings_operation(path, "/dsh-whale/usage-settings.json", "PUT", &patch)?;
    Ok(())
}

async fn probe_model(
    settings: &BackendSettings,
    path: &Path,
    model: &Value,
    history: &Value,
) -> Value {
    if model["provider"] == CODEX {
        return if history["status"] == "ok" || history["status"] == "partial" {
            json!({"ok":true,"detail":"Codex 本机会话统计可用"})
        } else {
            json!({"ok":false,"error":"本机统计已关闭或尚未观测到会话"})
        };
    }
    let tpl = templates()
        .into_iter()
        .find(|v| v["id"] == model["provider"])
        .unwrap_or(Value::Null);
    // 自定义非 GET 描述必须走配置的同一请求方法和 JSON 请求体。
    if !string(model, "provider").is_empty()
        && (!string(&tpl, "probeUrl").is_empty())
        && matches!(string(&model["balance"], "method"), "" | "GET")
    {
        let Some(profile) = profile_for(settings, model) else {
            return json!({"ok":false,"error":"所选供应商配置不存在"});
        };
        let raw = string(&tpl, "probeUrl");
        let auth = if tpl["quota"].is_object() {
            tpl["quota"]["auth"].clone()
        } else {
            tpl["balance"]["auth"].clone()
        };
        let descriptor = json!({"url":raw,"auth":auth,"queryKey":model["provider"]=="gemini"});
        return match fetch(
            &profile,
            &descriptor,
            &model["params"],
            &model["approvedOrigins"],
        )
        .await
        {
            Ok(_) => json!({"ok":true,"detail":"供应商接口连通"}),
            Err(err) => json!({"ok":false,"error":err.to_string()}),
        };
    }
    let observed = model_observation(settings, path, model, true).await;
    if observed["error"].is_null()
        && (observed["balance"].is_number() || observed["plan"]["ok"] == true)
    {
        json!({"ok":true,"detail":"监测接口可用"})
    } else {
        json!({"ok":false,"error":observed["error"].as_str().unwrap_or("供应商没有公开监测接口，可使用会话统计")})
    }
}

async fn models_payload(
    settings: &BackendSettings,
    path: &Path,
    history: &Value,
) -> anyhow::Result<Value> {
    let monitors = monitor_models(settings, path)?;
    let usage = read_usage(path)?;
    let observations = futures_util::future::join_all(
        monitors
            .iter()
            .map(|m| model_observation(settings, path, m, false)),
    )
    .await;
    let mut out = Vec::new();
    for (model, observation) in monitors.iter().zip(observations) {
        let mut entry = model.clone();
        merge(&mut entry, &observation);
        let id = string(model, "id");
        let per = usage["models"][id].clone();
        entry["settings"] = if id == CURRENT {
            json!({"alert":usage["alert"],"budget":usage["budget"],"quota":per["quota"]})
        } else {
            per.clone()
        };
        entry["quota"] = per["quota"].clone();
        if entry["quota"].is_object() {
            entry["quota"]["autoUsed"] = model_quota_used(model, &entry["quota"], history);
            let q = json!({"mode":"auto","reset":"daily"});
            entry["quota"]["autoToday"] = model_token_total(model, &q, history)
                .map(|v| json!(v))
                .unwrap_or(Value::Null);
        }
        if model["provider"] == CODEX {
            entry["codex"] = history
                .get("codex")
                .or_else(|| history.get("machineSummary"))
                .cloned()
                .unwrap_or_else(|| history.clone());
            entry["hasKey"] = json!(true);
            entry["error"] = Value::Null;
        }
        if !entry["todayUsage"].is_number() {
            if let Some((amount, partial)) = today_estimate(model, history) {
                entry["todayUsage"] = json!((amount * 100_000_000.0).round() / 100_000_000.0);
                entry["todayUsageCurrency"] = json!("CNY");
                entry["usageSource"] = json!("events");
                entry["usageLabel"] = json!(if partial {
                    "本地估算 · 部分记录"
                } else {
                    "本地估算"
                });
                entry["partialDay"] = json!(partial);
            } else {
                entry["todayUsage"] = Value::Null;
                entry["todayUsageCurrency"] = model["currency"].clone();
                entry["usageSource"] = json!("none");
                entry["usageLabel"] = json!("尚未配置计价");
            }
        }
        out.push(entry);
    }
    let mut all_profiles: Vec<Value> = settings
        .relay_profiles
        .iter()
        .map(|p| json!({"id":p.id,"name":p.name}))
        .collect();
    let active = settings.active_relay_profile();
    if !all_profiles.iter().any(|v| v["id"] == active.id) {
        all_profiles.push(json!({"id":active.id,"name":active.name}));
    }
    Ok(
        json!({"ok":true,"builtinId":CURRENT,"templates":templates(),"relayProfiles":all_profiles,"models":out,"codexStatsOn":history_enabled(path)}),
    )
}

fn save_model(settings: &BackendSettings, path: &Path, input: &Value) -> anyhow::Result<String> {
    if input
        .get("keyValue")
        .is_some_and(|v| !v.is_null() && v != "")
    {
        return Err(error(
            "请在 Codex++ 的供应商配置中保存密钥，此处只选择已有配置 ID",
        ));
    }
    let model = input.get("model").unwrap_or(input);
    let provider = string(model, "provider");
    let tpl = templates()
        .into_iter()
        .find(|v| string(v, "id") == provider)
        .ok_or_else(|| error("未知的厂商模板"))?;
    let id = if string(model, "id").is_empty() {
        format!("api_{}", uuid::Uuid::new_v4().simple())
    } else {
        string(model, "id").to_string()
    };
    if !id_is_valid(&id) {
        return Err(error("模型 ID 无效"));
    }
    if id == CODEX && provider != CODEX {
        return Err(error(
            "内置 Codex 统计模型不能改为其他供应商，可另添加监测模型",
        ));
    }
    let mut saved = json!({"id":id,"name":limited_name(model,"name",string(&tpl,"name"),40),"provider":provider,"currency":limited_name(model,"currency",string(&tpl,"currency"),3).to_uppercase(),"keyRef":string(model,"keyRef"),"builtin":id==CURRENT});
    if string(&saved, "currency").len() != 3
        || !string(&saved, "currency")
            .bytes()
            .all(|b| b.is_ascii_uppercase())
    {
        return Err(error("币种必须是三位大写代码"));
    }
    if provider != CODEX {
        let profile = profile_for(settings, &saved)
            .ok_or_else(|| error("请选择已有 Codex++ 供应商配置 ID"))?;
        saved["keyRef"] = json!(profile.id);
        // 仅保存动作可以批准模板的固定端点。当前供应商自动探测不会批准新域。
        let mut approved = Vec::new();
        for desc in [&tpl["balance"], &tpl["balance"]["usage"], &tpl["quota"]] {
            if let Ok(url) = reqwest::Url::parse(string(desc, "url")) {
                if url.scheme() == "https" {
                    approved.push(json!(url.origin().ascii_serialization()));
                }
            }
        }
        if let Ok(url) = reqwest::Url::parse(
            string(&tpl, "probeUrl")
                .replace("{key}", "placeholder")
                .as_str(),
        ) {
            if url.scheme() == "https" {
                approved.push(json!(url.origin().ascii_serialization()));
            }
        }
        if model["allowCustomHost"] == true {
            for desc in [
                &model["balance"],
                &model["balance"]["usage"],
                &model["quotaDesc"],
                &model["quota"],
            ] {
                if let Ok(url) = reqwest::Url::parse(string(desc, "url")) {
                    if url.scheme() == "https" {
                        approved.push(json!(url.origin().ascii_serialization()));
                    }
                }
            }
        }
        approved.sort_by_key(Value::to_string);
        approved.dedup();
        saved["approvedOrigins"] = json!(approved);
        saved["allowCustomHost"] = json!(model["allowCustomHost"] == true);
        for key in ["balance", "quotaDesc"] {
            let desc = if key == "quotaDesc" {
                model
                    .get(key)
                    .or_else(|| model.get("quota").filter(|v| !string(v, "url").is_empty()))
            } else {
                model.get(key)
            };
            if let Some(desc) = desc.filter(|v| v.is_object()) {
                for endpoint_desc in [desc, &desc["usage"]] {
                    let url = string(endpoint_desc, "url");
                    if !url.is_empty() {
                        endpoint_allowed(&profile, url, &saved["approvedOrigins"])?;
                    }
                }
                saved[key] = desc.clone();
            }
        }
    }
    if let Some(params) = model.get("params").filter(|v| v.is_object()) {
        if params.to_string().len() > 4096
            || params.as_object().unwrap().len() > 32
            || params
                .as_object()
                .unwrap()
                .values()
                .any(|v| v.is_object() || v.is_array())
        {
            return Err(error("请求参数仅支持最多 32 个标量字段"));
        }
        saved["params"] = params.clone();
    }
    if let Some(ids) = model.get("matchIds").filter(|v| v.is_array()) {
        if ids.as_array().unwrap().len() > 32
            || ids
                .as_array()
                .unwrap()
                .iter()
                .any(|v| v.as_str().is_none_or(|s| s.len() > 200))
        {
            return Err(error("模型匹配规则无效"));
        }
        saved["matchIds"] = ids.clone();
    }
    if let Some(price) = model.get("price").filter(|v| v.is_object()) {
        let mut normalized = price.clone();
        for key in ["hit", "miss", "out", "rate"] {
            if normalized[key]
                .as_str()
                .is_some_and(|s| s.trim().is_empty())
            {
                normalized[key] = Value::Null;
            }
        }
        for key in ["hit", "miss", "out"] {
            if !normalized[key].is_null()
                && !number(&normalized[key]).is_some_and(|v| (0.0..=1_000_000.0).contains(&v))
            {
                return Err(error("单价必须为 0–1000000 的有限数（每百万 token）"));
            }
        }
        let currency = string(price, "cur");
        if !matches!(currency, "" | "USD" | "CNY") {
            return Err(error("计价币种只支持 USD 或 CNY"));
        }
        if !normalized["rate"].is_null()
            && !number(&normalized["rate"]).is_some_and(|v| v > 0.0 && v <= 1000.0)
        {
            return Err(error("汇率必须为 0–1000 的正数"));
        }
        if ["hit", "miss", "out"]
            .iter()
            .any(|k| !normalized[*k].is_null())
        {
            saved["price"] = normalized;
        }
    }
    let mut rows = read_doc(path, "models", json!([]))?
        .as_array()
        .cloned()
        .unwrap_or_default();
    if let Some(i) = rows
        .iter()
        .position(|v| v["id"] == id && (id != CURRENT || v["keyRef"] == saved["keyRef"]))
    {
        rows[i] = saved;
    } else {
        if rows.len() >= 32 {
            return Err(error("监测模型最多 32 项"));
        }
        rows.push(saved);
    }
    write_doc(path, "models", &json!(rows))?;
    Ok(id)
}

fn remove_model_references(value: &mut Value, id: &str) -> usize {
    let mut removed = 0;
    match value {
        Value::Array(rows) => {
            rows.retain_mut(|row| {
                if row["modelId"] == id || row["module"]["modelId"] == id {
                    removed += 1;
                    false
                } else {
                    removed += remove_model_references(row, id);
                    true
                }
            });
        }
        Value::Object(fields) => {
            for child in fields.values_mut() {
                removed += remove_model_references(child, id);
            }
        }
        _ => {}
    }
    removed
}

fn delete_model(path: &Path, id: &str) -> anyhow::Result<usize> {
    if !id_is_valid(id) || matches!(id, CURRENT | CODEX) {
        return Err(error("Codex 内置统计与当前供应商不能删除"));
    }
    let mut rows = read_doc(path, "models", json!([]))?
        .as_array()
        .cloned()
        .unwrap_or_default();
    rows.retain(|v| string(v, "id") != id);
    let mut usage = read_doc(path, "usage", json!({}))?;
    if let Some(models) = usage["models"].as_object_mut() {
        models.remove(id);
    }
    let mut bubble = read_doc(path, "bubble", Value::Null)?;
    let count = remove_model_references(&mut bubble, id);
    let mut connection = open(path)?;
    let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    for (name, value) in [
        ("models", json!(rows)),
        ("usage", usage),
        ("bubble", bubble),
    ] {
        tx.execute("INSERT INTO whale_full_documents(name,json) VALUES(?1,?2) ON CONFLICT(name) DO UPDATE SET json=excluded.json",params![name,value.to_string()])?;
    }
    tx.commit()?;
    Ok(count)
}

fn today_days() -> anyhow::Result<Vec<String>> {
    let connection = Connection::open_in_memory()?;
    let mut days = Vec::new();
    for i in 0..7 {
        days.push(connection.query_row(
            "SELECT date('now','localtime',?1)",
            [format!("-{i} day")],
            |r| r.get(0),
        )?);
    }
    Ok(days)
}

fn peak_info(model: &Value, seconds: i64) -> anyhow::Result<Value> {
    let built_in = model["provider"] == "deepseek";
    let schedule = &model["price"]["peakSchedule"];
    if !built_in && !schedule.is_object() {
        return Ok(
            json!({"peakSupported":false,"isPeak":null,"peakNextChangeAt":null,"peakHolidays":[],"peakMessage":"当前供应商未配置峰谷时段"}),
        );
    }
    let calendar: Value = serde_json::from_str(include_str!(
        "../../../assets/inject/upstream/whale-widget/peak-calendar.json"
    ))?;
    let holidays = if built_in {
        calendar["holidays"].as_array().cloned().unwrap_or_default()
    } else {
        schedule["holidays"].as_array().cloned().unwrap_or_default()
    };
    let connection = Connection::open_in_memory()?;
    let year: String = connection.query_row(
        "SELECT strftime('%Y',?1,'unixepoch','+8 hours')",
        [seconds],
        |r| r.get(0),
    )?;
    if built_in && year != "2026" {
        return Ok(
            json!({"peakSupported":false,"isPeak":null,"peakNextChangeAt":null,"peakHolidays":holidays,"peakMessage":"此版本尚未配置当前年份的法定节假日时段表"}),
        );
    }
    let windows = if built_in {
        vec![(9i64, 12i64), (14, 18)]
    } else {
        schedule["windows"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|v| Some((v[0].as_i64()?, v[1].as_i64()?)))
            .filter(|(a, b)| *a >= 0 && *a < *b && *b <= 24)
            .take(8)
            .collect::<Vec<_>>()
    };
    if windows.is_empty() {
        return Ok(
            json!({"peakSupported":false,"isPeak":null,"peakNextChangeAt":null,"peakHolidays":holidays,"peakMessage":"峰谷时段配置无效"}),
        );
    }
    let weekdays = if built_in {
        vec![1, 2, 3, 4, 5]
    } else {
        schedule["weekdays"]
            .as_array()
            .map(|xs| {
                xs.iter()
                    .filter_map(Value::as_i64)
                    .filter(|n| (0..=6).contains(n))
                    .collect()
            })
            .unwrap_or(vec![1, 2, 3, 4, 5])
    };
    let weekend_effective: i64 =
        connection.query_row("SELECT unixepoch('2026-08-23 00:00:00+08:00')", [], |r| {
            r.get(0)
        })?;
    let holiday_effective: i64 =
        connection.query_row("SELECT unixepoch('2026-09-19 00:00:00+08:00')", [], |r| {
            r.get(0)
        })?;
    let is_peak = |time: i64| -> rusqlite::Result<bool> {
        let (day,dow,hour):(String,i64,i64)=connection.query_row("SELECT date(?1,'unixepoch','+8 hours'),CAST(strftime('%w',?1,'unixepoch','+8 hours') AS INTEGER),CAST(strftime('%H',?1,'unixepoch','+8 hours') AS INTEGER)",[time],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?)))?;
        Ok((!built_in || time >= weekend_effective)
            .then(|| weekdays.contains(&dow))
            .unwrap_or(true)
            && !((!built_in || time >= holiday_effective) && holidays.iter().any(|v| v == &day))
            && windows
                .iter()
                .any(|(start, end)| hour >= *start && hour < *end))
    };
    let current = is_peak(seconds)?;
    let day0 = (seconds + 8 * 3600).div_euclid(86400) * 86400;
    let mut edges = vec![0];
    for (start, end) in &windows {
        edges.push(*start);
        edges.push(*end);
    }
    edges.sort();
    edges.dedup();
    let mut next = None;
    for day in 0..=12 {
        for hour in &edges {
            let candidate = day0 + day * 86400 + hour * 3600 - 8 * 3600;
            if candidate > seconds + 1 && is_peak(candidate)? != current {
                next = Some(candidate);
                break;
            }
        }
        if next.is_some() {
            break;
        }
    }
    Ok(
        json!({"peakSupported":true,"isPeak":current,"peakNextChangeAt":next,"peakHolidays":holidays,"peakTimezone":"Asia/Shanghai","peakRuleSource":if built_in {"DeepSeek 参考项目固定版本时段规则"} else {"用户配置的时段规则"}}),
    )
}

fn current_account(settings: &BackendSettings, path: &Path) -> anyhow::Result<Option<String>> {
    let model = monitor_models(settings, path)?
        .into_iter()
        .find(|m| m["id"] == CURRENT)
        .unwrap();
    let Some(profile) = profile_for(settings, &model) else {
        return Ok(None);
    };
    let tpl = templates()
        .into_iter()
        .find(|v| v["id"] == model["provider"])
        .unwrap_or(Value::Null);
    let mut balance = tpl["balance"].clone();
    if balance.is_null() {
        balance = json!({});
    }
    merge_nonempty(&mut balance, &model["balance"]);
    if string(&balance, "url").is_empty() {
        return Ok(None);
    }
    Ok(Some(format!(
        "{:x}",
        Sha256::digest(
            json!([
                crate::relay_config::relay_profile_base_url(&profile),
                crate::relay_config::relay_profile_api_key(&profile),
                balance,
                model["currency"]
            ])
            .to_string()
            .as_bytes()
        )
    )))
}

fn records(settings: &BackendSettings, path: &Path, history: &Value) -> anyhow::Result<Value> {
    let active = current_account(settings, path)?;
    let observed = crate::whale::daily_records(path, None)?;
    let days7 = today_days()?;
    let today = &days7[0];
    let mut dates: BTreeMap<String, Vec<Value>> = BTreeMap::new();
    for row in observed {
        dates
            .entry(string(&row, "day").into())
            .or_default()
            .push(row);
    }
    if let Some(days) = history["days"].as_array() {
        for day in days {
            dates.entry(string(day, "date").into()).or_default();
        }
    }
    for day in &days7 {
        dates.entry(day.clone()).or_default();
    }
    let mut all = Vec::new();
    for (date, mut books) in dates.into_iter().rev() {
        for book in &mut books {
            book["active"] = json!(active.as_deref() == Some(string(book, "accountId")));
        }
        books.sort_by_key(|book| std::cmp::Reverse(book["active"] == true));
        let source_day = history["days"]
            .as_array()
            .and_then(|xs| xs.iter().find(|v| v["date"] == date));
        let costs = source_day
            .and_then(|v| v["estimatedCosts"].as_array())
            .cloned()
            .unwrap_or_default();
        let estimate = costs
            .iter()
            .find(|v| v["currency"] == "USD")
            .or_else(|| costs.first());
        let mut row=books.first().cloned().unwrap_or_else(||json!({"day":date,"date":date,"amount":estimate.and_then(|v|number(&v["amount"])),"currency":estimate.map(|v|v["currency"].clone()).unwrap_or(json!("USD")),
            "source":"events","label":if estimate.is_some() {"本地估算"} else {"尚未配置计价"},"partialDay":true}));
        let models=source_day.and_then(|v|v["models"].as_array()).cloned().unwrap_or_default().into_iter().map(|m|json!({"model":m["model"],"cost":m["estimatedCosts"].as_array().and_then(|xs|xs.first()).and_then(|v|number(&v["amount"])),"source":"events","currency":m["estimatedCosts"].as_array().and_then(|xs|xs.first()).map(|v|v["currency"].clone()).unwrap_or(json!("USD")),"usage":m["usage"]})).collect::<Vec<_>>();
        if !books.is_empty() && row["active"] != true {
            row["source"] = json!("balance-observed-other-account");
            row["label"] = json!("已观测消费 · 历史账户");
        }
        let other = books
            .iter()
            .filter(|b| b["accountId"] != row["accountId"] && b["currency"] == row["currency"])
            .filter_map(|b| number(&b["amount"]))
            .sum::<f64>();
        row["otherBookAmount"] = json!(other);
        row["bookCount"] = json!(books.len());
        if books.len() > 1 {
            row["historyHint"] = json!(format!(
                "另有 {} 个历史记账本，账户与币种分别保留",
                books.len() - 1
            ));
        }
        row["date"] = json!(date);
        row["total"] = row["amount"].clone();
        row["models"] = json!(models);
        row["modelTotal"] = estimate
            .and_then(|v| number(&v["amount"]))
            .map(|v| json!(v))
            .unwrap_or(Value::Null);
        row["modelCurrency"] = estimate
            .map(|v| v["currency"].clone())
            .unwrap_or(json!("USD"));
        row["bookBreakdown"] = json!(books);
        all.push(row);
    }
    let today_row = all
        .iter()
        .find(|r| r["date"] == *today)
        .cloned()
        .unwrap_or(Value::Null);
    let seven = days7
        .iter()
        .filter_map(|day| all.iter().find(|r| r["date"] == *day).cloned())
        .collect::<Vec<_>>();
    let mut totals: BTreeMap<String, f64> = BTreeMap::new();
    for row in &seven {
        if let Some(amount) = number(&row["total"]) {
            *totals.entry(string(row, "currency").into()).or_default() += amount;
        }
    }
    let events=history["records"].as_array().cloned().unwrap_or_default().into_iter().map(|r|json!({"ts":r["updatedAt"],"day":r["date"],"model":r["model"],"cost":r["estimatedCost"]["amount"],"currency":r["estimatedCost"]["currency"],"tokens":r["usage"]["totalTokens"],"turn":r["turnId"]})).collect::<Vec<_>>();
    Ok(
        json!({"ok":true,"version":"codex++","today":today_row,"days7":seven,"total7":totals.get(string(&today_row,"currency")),"total7Currency":today_row["currency"],"total7ByCurrency":totals,"all":{"days":all,"events":events},"settings":read_usage(path)?,"complete":history["complete"],"scan":history["scan"]}),
    )
}

async fn dispatch(
    settings: &BackendSettings,
    path: &Path,
    request: &Value,
    history: &Value,
) -> anyhow::Result<Value> {
    let route = string(request, "path");
    let method = match string(request, "method") {
        "" | "GET" => "GET",
        "POST" => "POST",
        "PUT" => "PUT",
        _ => return Ok(failure(405, "接口仅支持 GET、POST 或 PUT")),
    };
    let query = &request["query"];
    let body = &request["body"];
    if route.len() > 100
        || query.to_string().len() > 4096
        || body.to_string().len() > 30 * 1024 * 1024
    {
        return Ok(failure(413, "请求超过挂件接口大小限制"));
    }
    match route {
        "/dsh-whale/size.json" | "/dsh-whale/bubble.json" | "/dsh-whale/usage-settings.json" => {
            settings_operation(path, route, method, body)
        }
        "/dsh-whale/roles.json"
        | "/dsh-whale/role-pin.json"
        | "/dsh-whale/role-delete.json"
        | "/dsh-whale/role-image.png"
        | "/dsh-whale/bubble-imgs.json"
        | "/dsh-whale/bubble-img-upload.json"
        | "/dsh-whale/bubble-img.png"
        | "/dsh-whale/audio.json"
        | "/dsh-whale/audio-fragment.wav"
        | "/dsh-whale/sound/press.mp3"
        | "/dsh-whale/sound/release.mp3" => media_operation(path, route, method, query, body),
        "/dsh-whale/usage-records.json" => Ok(response(200, records(settings, path, history)?)),
        "/dsh-whale/api-models.json" => {
            let mut details = json!({});
            if method != "GET" {
                match string(body, "action") {
                    "probe" => {
                        let model = monitor_models(settings, path)?
                            .into_iter()
                            .find(|v| v["id"] == body["id"])
                            .ok_or_else(|| error("监测模型不存在"))?;
                        return Ok(response(
                            200,
                            probe_model(settings, path, &model, history).await,
                        ));
                    }
                    "delete" => {
                        details["removedModules"] = json!(delete_model(path, string(body, "id"))?);
                    }
                    "set-key" | "delete-key" => {
                        return Err(error(
                            "请在 Codex++ 的供应商配置中管理密钥，挂件只复用已有配置",
                        ));
                    }
                    "model-settings" => model_usage_patch(settings, path, body, history)?,
                    "" | "save" => {
                        details["id"] = json!(save_model(settings, path, body)?);
                        details["keySaved"] = json!(false);
                    }
                    _ => return Err(error("未知模型操作")),
                }
            }
            let mut payload = models_payload(settings, path, history).await?;
            merge(&mut payload, &details);
            Ok(response(200, payload))
        }
        "/dsh-whale/balance.json" => {
            let model = monitor_models(settings, path)?
                .into_iter()
                .find(|v| v["id"] == CURRENT)
                .unwrap();
            let observed = model_observation(settings, path, &model, query["refresh"] == "1").await;
            let mut value = json!({"ok":observed["balance"].is_number(),"totalBalance":observed["balance"],"currency":model["currency"],"todayUsage":observed["todayUsage"],"todayUsageCurrency":observed["todayUsageCurrency"],"bonusBalance":observed["bonusBalance"],"rechargeBalance":observed["rechargeBalance"],"usageLabel":observed["usageLabel"],"usageSource":observed["usageSource"],"accounting":observed["accounting"],"stale":observed["stale"],"error":observed["error"],"updatedAt":now()});
            merge(&mut value, &peak_info(&model, (now() / 1000) as i64)?);
            Ok(response(200, value))
        }
        "/dsh-whale/balance-adjustments.json" => {
            let model_id = string(query, "modelId");
            let model = monitor_models(settings, path)?
                .into_iter()
                .find(|v| string(v, "id") == model_id)
                .ok_or_else(|| error("监测模型不存在"))?;
            let observed = model_observation(settings, path, &model, true).await;
            if observed["canAdjustBalance"] != true {
                return Ok(failure(403, "该供应商没有可校正的余额观测"));
            }
            let account = string(&observed, "accountId");
            let currency = string(&model, "currency");
            if method != "GET" {
                if string(body, "modelId") != model_id {
                    return Ok(failure(403, "校正请求的模型不匹配"));
                }
                if body["day"] == crate::whale::local_day()?
                    && (observed["stale"] == true || !observed["balance"].is_number())
                {
                    return Ok(failure(503, "暂时无法刷新余额，请稍后再保存校正"));
                }
                let summary = crate::whale::reconcile(path, account, currency, body)?;
                let prefix = format!("{}:", path.to_string_lossy());
                FETCH_CACHE
                    .lock()
                    .await
                    .retain(|key, _| !key.starts_with(&prefix));
                return Ok(response(200, json!({"ok":true,"summary":summary})));
            }
            Ok(response(
                200,
                json!({"ok":true,"days":crate::whale::daily_records(path,Some(account))?,"today":crate::whale::local_day()?,"fresh":observed["stale"]!=true && observed["balance"].is_number(),"error":observed["error"]}),
            ))
        }
        _ => Ok(failure(404, "未知挂件接口")),
    }
}

pub async fn handle(
    settings: &BackendSettings,
    path: PathBuf,
    request: Value,
    history: Value,
) -> Value {
    if !crate::whale::enabled(settings) {
        return failure(403, "Codex 用量挂件已关闭");
    }
    let _guard = if matches!(string(&request, "method"), "POST" | "PUT") {
        Some(WRITE_LOCK.lock().await)
    } else {
        None
    };
    match dispatch(settings, &path, &request, &history).await {
        Ok(value) => value,
        Err(err) => {
            // 仅有限的业务提示可透传；数据库、文件或网络错误不得带路径、响应正文或凭据。
            let message = err.to_string();
            let safe = if err.is::<rusqlite::Error>()
                || err.is::<std::io::Error>()
                || err.is::<serde_json::Error>()
            {
                "挂件数据暂不可用，原记录已保留"
            } else {
                message.as_str()
            };
            failure(400, safe)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;
    fn enabled_settings() -> BackendSettings {
        let mut settings = BackendSettings::default();
        settings.codex_app_whale_widget_enabled = true;
        settings
    }
    #[tokio::test]
    async fn disabled_api_has_no_io_and_unknown_routes_are_closed() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("ledger.db");
        let result = handle(
            &BackendSettings::default(),
            path.clone(),
            json!({"path":"/dsh-whale/size.json"}),
            json!({}),
        )
        .await;
        assert_eq!(result["status"], 403);
        assert!(!path.exists());
        let result = handle(
            &enabled_settings(),
            path.clone(),
            json!({"path":"../../auth.json"}),
            json!({}),
        )
        .await;
        assert_eq!(result["status"], 404);
        assert!(!path.exists());
    }
    #[tokio::test]
    async fn size_patch_preserves_unmentioned_settings_and_bubble_is_persistent() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("ledger.db");
        let settings = enabled_settings();
        handle(&settings,path.clone(),json!({"path":"/dsh-whale/size.json","method":"PUT","body":{"scale":1.5,"sound":false,"scrollGapPx":31}}),json!({})).await;
        let result = handle(
            &settings,
            path.clone(),
            json!({"path":"/dsh-whale/size.json","method":"PUT","body":{"scale":2.0}}),
            json!({}),
        )
        .await;
        assert_eq!(result["body"]["sound"], false);
        assert_eq!(result["body"]["scrollGapPx"], 31.0);
        let cfg = json!({"items":[{"type":"text","text":"测试"}],"lib":[],"tapAdvance":true});
        handle(
            &settings,
            path.clone(),
            json!({"path":"/dsh-whale/bubble.json","method":"POST","body":cfg}),
            json!({}),
        )
        .await;
        let read = handle(
            &settings,
            path,
            json!({"path":"/dsh-whale/bubble.json"}),
            json!({}),
        )
        .await;
        assert_eq!(read["body"]["config"]["items"], cfg["items"]);
    }
    #[test]
    fn media_requires_matching_mime_and_wav_headers() {
        assert!(
            decode_media(
                &format!(
                    "data:image/png;base64,{}",
                    STANDARD.encode(b"<svg>bad</svg>")
                ),
                "role"
            )
            .is_err()
        );
        assert!(decode_media("data:image/svg+xml;base64,PHN2Zz4=", "role").is_err());
        assert!(decode_media("data:audio/wav;base64,aGVsbG8=", "audio").is_err());
    }
    #[test]
    fn template_endpoints_never_move_profile_credentials_to_another_origin() {
        let profile = RelayProfile {
            base_url: "https://relay.example/v1".into(),
            ..RelayProfile::default()
        };
        assert!(endpoint(&profile, "/balance").is_ok());
        assert!(endpoint(&profile, "https://relay.example/balance").is_ok());
        for bad in [
            "https://openrouter.ai/api/v1/credits",
            "https://relay.example.evil/balance",
            "https://user@relay.example/balance",
            "file:///tmp/auth",
            "https://relay.example/?key={key}",
        ] {
            assert!(endpoint(&profile, bad).is_err(), "{bad}");
        }
    }
    #[test]
    fn quota_windows_use_type_and_unit_and_keep_distinct_reset_times() {
        let data = json!({"data":{"limits":[{"type":"TIME_LIMIT","unit":3,"percentage":88},{"type":"CREDIT_LIMIT","unit":6,"percentage":40,"nextResetTime":1800000000},{"type":"TOKENS_LIMIT","unit":3,"percentage":20,"nextResetTime":1790000000}]}});
        let tpl = templates()
            .into_iter()
            .find(|v| v["id"] == "zhipu_glm_coding")
            .unwrap();
        let parsed = quota_payload(&data, &tpl["quota"]["json"]).unwrap();
        assert_eq!(parsed["windows"][0]["percent"], 20.0);
        assert_eq!(parsed["windows"][1]["percent"], 40.0);
        assert_eq!(parsed["windows"][0]["resetAt"], 1790000000);
        assert_eq!(parsed["usedPct"], 20.0);
        assert_eq!(parsed["remainPct"], 80.0);
        assert_eq!(parsed["weeklyUsedPct"], 40.0);
        assert_eq!(parsed["windows"][1]["usedPct"], 40.0);
    }

    #[test]
    fn peak_calendar_tracks_real_workday_holiday_and_weekend_changes() {
        let connection = Connection::open_in_memory().unwrap();
        let seconds = |text: &str| {
            connection
                .query_row("SELECT unixepoch(?1)", [text], |r| r.get::<_, i64>(0))
                .unwrap()
        };
        let model = json!({"provider":"deepseek"});
        let workday = peak_info(&model, seconds("2026-10-08 10:00:00+08:00")).unwrap();
        assert_eq!(workday["isPeak"], true);
        assert_eq!(
            workday["peakNextChangeAt"],
            seconds("2026-10-08 12:00:00+08:00")
        );
        let holiday = peak_info(&model, seconds("2026-10-07 10:00:00+08:00")).unwrap();
        assert_eq!(holiday["isPeak"], false);
        assert_eq!(
            holiday["peakNextChangeAt"],
            seconds("2026-10-08 09:00:00+08:00")
        );
        let weekend = peak_info(&model, seconds("2026-10-10 10:00:00+08:00")).unwrap();
        assert_eq!(weekend["isPeak"], false);
        assert_eq!(
            weekend["peakNextChangeAt"],
            seconds("2026-10-12 09:00:00+08:00")
        );
        let other = peak_info(
            &json!({"provider":"custom"}),
            seconds("2026-10-08 10:00:00+08:00"),
        )
        .unwrap();
        assert_eq!(other["peakSupported"], false);
        assert!(other["isPeak"].is_null());
    }

    #[test]
    fn media_limits_match_source_role_twenty_and_small_media_eight_mib() {
        let mut bytes = vec![0u8; MAX_ROLE_MEDIA];
        bytes[..6].copy_from_slice(b"GIF89a");
        bytes[6..10].copy_from_slice(&[1, 0, 1, 0]);
        *bytes.last_mut().unwrap() = b';';
        let data = format!("data:image/gif;base64,{}", STANDARD.encode(&bytes));
        assert!(decode_media(&data, "role").is_ok());
        assert!(decode_media(&data, "image").is_err());
        bytes.push(b';');
        let data = format!("data:image/gif;base64,{}", STANDARD.encode(&bytes));
        assert!(decode_media(&data, "role").is_err());
        assert!(
            decode_media(
                &format!(
                    "data:image/png;base64,{}",
                    STANDARD.encode(b"\x89PNG\r\n\x1a\n")
                ),
                "role"
            )
            .is_err()
        );
    }

    #[tokio::test]
    async fn http_query_key_is_encoded_and_non_get_body_preserves_placeholders_without_leaks() {
        use wiremock::matchers::{body_json, header, method, path, query_param};
        use wiremock::{Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        let key = "fake &+=?#/中文";
        let profile = RelayProfile {
            base_url: format!("{}/v1", server.uri()),
            api_key: key.into(),
            ..RelayProfile::default()
        };
        Mock::given(method("GET"))
            .and(path("/models"))
            .and(query_param("key", key))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"models":[]})))
            .expect(1)
            .mount(&server)
            .await;
        let descriptor =
            json!({"url":format!("{}/models?key={{key}}",server.uri()),"auth":"","queryKey":true});
        assert!(
            fetch(&profile, &descriptor, &Value::Null, &Value::Null)
                .await
                .is_ok()
        );
        let descriptor = json!({"url":"/balance","auth":"Bearer {key}","method":"PATCH","body":"{\"uuid\":\"{uuid}\",\"key\":\"{key}\",\"nested\":[\"{uuid}\"]}"});
        // HTTP 请求头限制为 Latin-1，给 body 的 quote/反斜线转义单独使用 ASCII 测试密钥。
        let profile = RelayProfile {
            api_key: "fake-ASCII-key".into(),
            ..profile
        };
        // 重新使用一个匹配 ASCII header 的端点，不把参数替换成未经 JSON 转义的文本。
        Mock::given(method("PATCH"))
            .and(path("/ascii-balance"))
            .and(header("authorization", "Bearer fake-ASCII-key"))
            .and(body_json(
                json!({"uuid":"account\"test","key":"fake-ASCII-key","nested":["account\"test"]}),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"remaining":2})))
            .expect(1)
            .mount(&server)
            .await;
        let mut descriptor = descriptor;
        descriptor["url"] = json!("/ascii-balance");
        assert_eq!(
            fetch(
                &profile,
                &descriptor,
                &json!({"uuid":"account\"test"}),
                &Value::Null
            )
            .await
            .unwrap()["remaining"],
            2
        );
        Mock::given(method("GET"))
            .and(path("/failure"))
            .respond_with(
                ResponseTemplate::new(401).set_body_string("fake-ASCII-key secret response"),
            )
            .mount(&server)
            .await;
        let failure = fetch(
            &profile,
            &json!({"url":"/failure"}),
            &Value::Null,
            &Value::Null,
        )
        .await
        .unwrap_err()
        .to_string();
        assert!(!failure.contains("fake-ASCII-key"));
        assert!(!failure.contains("secret response"));
    }

    #[tokio::test]
    async fn missing_configured_used_field_never_becomes_zero_or_creates_balance_ledger() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/balance"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"total":100})))
            .expect(1)
            .mount(&server)
            .await;
        let profile = RelayProfile {
            id: "fixture-profile".into(),
            base_url: format!("{}/v1", server.uri()),
            api_key: "fake-key".into(),
            ..RelayProfile::default()
        };
        let mut settings = enabled_settings();
        settings.relay_profiles = vec![profile];
        let temp = TempDir::new().unwrap();
        let ledger = temp.path().join("ledger.db");
        let result=model_observation(&settings,&ledger,&json!({"id":"test-model","keyRef":"fixture-profile","provider":"custom","currency":"USD","balance":{"url":"/balance","json":{"total":"total","used":"missing"}}}),true).await;
        assert!(result["balance"].is_null());
        assert!(result["error"].is_string());
        assert!(!ledger.exists());
    }

    #[test]
    fn token_price_fallback_uses_only_matching_models_and_marks_unknown_records() {
        let model = json!({"provider":"custom","matchIds":["gpt"]});
        let today = crate::whale::local_day().unwrap();
        let history = json!({"complete":true,"days":[{"date":today,"models":[{"model":"gpt-6-test","estimatedCosts":[{"currency":"CNY","amount":2.5}],"unpricedRecords":1},{"model":"other-model","estimatedCosts":[{"currency":"CNY","amount":9}]}]}]});
        assert_eq!(today_estimate(&model, &history), Some((2.5, true)));
        assert!(
            today_estimate(
                &json!({"provider":"custom","matchIds":["unknown"]}),
                &history
            )
            .is_none()
        );
    }

    #[tokio::test]
    async fn deleting_monitor_removes_its_settings_and_nested_bubble_references_atomically() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("ledger.db");
        let settings = enabled_settings();
        save_model(
            &settings,
            &path,
            &json!({"provider":"codex","id":"monitor-a","name":"A"}),
        )
        .unwrap();
        write_doc(&path,"usage",&json!({"models":{"monitor-a":{"alert":{"below":5}},"keep-me":{"budget":{"amount":8}}}})).unwrap();
        write_doc(&path,"bubble",&json!({"v":1,"items":[{"type":"parallel","children":[{"type":"quota","modelId":"monitor-a"},{"type":"text","text":"保留"}]}],"lib":[{"id":"a","module":{"type":"today","modelId":"monitor-a"}}],"tapAdvance":true})).unwrap();
        assert_eq!(delete_model(&path, "monitor-a").unwrap(), 2);
        assert!(
            read_doc(&path, "usage", Value::Null).unwrap()["models"]
                .get("monitor-a")
                .is_none()
        );
        assert_eq!(
            read_doc(&path, "usage", Value::Null).unwrap()["models"]["keep-me"]["budget"]["amount"],
            8
        );
        let bubble = read_doc(&path, "bubble", Value::Null).unwrap();
        assert_eq!(bubble["items"][0]["children"].as_array().unwrap().len(), 1);
        assert!(bubble["lib"].as_array().unwrap().is_empty());
    }
}
