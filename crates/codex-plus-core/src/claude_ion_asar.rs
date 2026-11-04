//! 新版 Claude 把本地界面打包成 `ion-dist.asar`（#2884）：在副本里就地改写这个包，
//! 装入中文词表、放开语言协商白名单并登记 `i18n-catalogs`，与目录形态的
//! `build-copy.ps1` / `build-copy.py` 做同样的变换。官方安装从不经过这里。

use std::path::Path;

use anyhow::{Context, Result, bail, ensure};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};

const BLOCK_SIZE: usize = 4 * 1024 * 1024;
const LANG_OLD: &str = r#"["en-US","de-DE","fr-FR","ko-KR","ja-JP","es-419","es-ES","it-IT","hi-IN","pt-BR","id-ID"]"#;
const LANG_NEW: &str = r#"["en-US","de-DE","fr-FR","ko-KR","ja-JP","es-419","es-ES","it-IT","hi-IN","pt-BR","id-ID","zh-Hans"]"#;

/// 文件内容：沿用原包里的字节区间，或者是改写 / 新增后的内容。
enum Content {
    Original { offset: usize, size: usize },
    Replaced(Vec<u8>),
}

struct Asar {
    header: Value,
    bytes: Vec<u8>,
    data_start: usize,
}

impl Asar {
    fn parse(bytes: Vec<u8>) -> Result<Self> {
        ensure!(bytes.len() >= 16, "ion-dist.asar 文件过短，格式无法识别");
        let word = |at: usize| u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap()) as usize;
        let (header_size, json_len) = (word(4), word(12));
        ensure!(
            16 + json_len <= bytes.len() && 8 + header_size <= bytes.len() && 16 + json_len <= 8 + header_size,
            "ion-dist.asar 头部损坏"
        );
        let header = serde_json::from_slice(&bytes[16..16 + json_len]).context("ion-dist.asar 头部不是合法 JSON")?;
        Ok(Self { header, bytes, data_start: 8 + header_size })
    }

    fn blob(&self, meta: &Value) -> Result<&[u8]> {
        ensure!(meta.get("unpacked").and_then(Value::as_bool) != Some(true), "暂不支持带 unpacked 文件的 ion-dist.asar");
        let offset: usize = meta["offset"].as_str().context("ion-dist.asar 条目缺少 offset")?.parse()?;
        let size = meta["size"].as_u64().context("ion-dist.asar 条目缺少 size")? as usize;
        let start = self.data_start + offset;
        self.bytes.get(start..start + size).context("ion-dist.asar 条目越界")
    }
}

/// 按遍历顺序列出所有文件条目的路径（`a/b/c.js`）。
fn file_paths(node: &Value, prefix: &str, out: &mut Vec<String>) {
    let Some(files) = node.get("files").and_then(Value::as_object) else { return };
    for (name, child) in files {
        let path = if prefix.is_empty() { name.clone() } else { format!("{prefix}/{name}") };
        if child.get("files").is_some() {
            file_paths(child, &path, out);
        } else {
            out.push(path);
        }
    }
}

fn entry<'a>(root: &'a Value, path: &str) -> Option<&'a Value> {
    path.split('/').try_fold(root, |node, name| node.get("files")?.get(name))
}

/// 取（必要时建好目录）`path` 的文件条目。
fn entry_mut<'a>(root: &'a mut Value, path: &str) -> Result<&'a mut Value> {
    let mut node = root;
    for name in path.split('/') {
        let files = node
            .as_object_mut()
            .context("ion-dist.asar 头部结构异常")?
            .entry("files")
            .or_insert_with(|| Value::Object(Map::new()))
            .as_object_mut()
            .context("ion-dist.asar 头部结构异常")?;
        node = files.entry(name.to_string()).or_insert_with(|| Value::Object(Map::new()));
    }
    Ok(node)
}

fn integrity(bytes: &[u8]) -> Value {
    let hex = |data: &[u8]| Sha256::digest(data).iter().map(|byte| format!("{byte:02x}")).collect::<String>();
    let blocks: Vec<String> = bytes.chunks(BLOCK_SIZE).map(hex).collect();
    json!({ "algorithm": "SHA256", "hash": hex(bytes), "blockSize": BLOCK_SIZE, "blocks": blocks })
}

/// 把 `<meta name="i18n-catalogs" …>` 换成（或插入）登记了全部词表的那一行。
fn register_catalogs(html: &str, langs: &[String]) -> Result<String> {
    let catalogs = langs.iter().map(|lang| format!("{lang}:1.1.1")).collect::<Vec<_>>().join(",");
    let meta = format!(r#"<meta name="i18n-catalogs" data-i18n-catalogs="{catalogs}">"#);
    let mut from = 0;
    while let Some(found) = html[from..].find("<meta") {
        let start = from + found;
        let Some(len) = html[start..].find('>') else { break };
        let tag = &html[start..start + len + 1];
        if tag.contains(r#"name="i18n-catalogs""#) || tag.contains("name='i18n-catalogs'") {
            return Ok(format!("{}{meta}{}", &html[..start], &html[start + len + 1..]));
        }
        from = start + len + 1;
    }
    ensure!(html.contains("</head>"), "本地界面的 HTML 格式已变化");
    Ok(html.replacen("</head>", &format!("{meta}</head>"), 1))
}

/// 改写副本里的 `ion-dist.asar`：`catalogs` 为 `(i18n 下的相对路径, 内容)`。
/// 先写临时文件再替换，失败时原文件不动。
pub(crate) fn localize_ion_asar(path: &Path, catalogs: &[(&str, &[u8])]) -> Result<()> {
    let asar = Asar::parse(std::fs::read(path).with_context(|| format!("无法读取 {}", path.display()))?)?;
    let mut header = asar.header.clone();
    let mut paths = Vec::new();
    file_paths(&header, "", &mut paths);
    let mut contents: std::collections::HashMap<String, Vec<u8>> = std::collections::HashMap::new();

    let mut matches = 0;
    for path in paths.iter().filter(|path| path.starts_with("assets/") && path.ends_with(".js")) {
        let blob = asar.blob(entry(&header, path).context("ion-dist.asar 条目缺失")?)?;
        let Ok(text) = std::str::from_utf8(blob) else { continue };
        if text.contains(LANG_OLD) {
            contents.insert(path.clone(), text.replace(LANG_OLD, LANG_NEW).into_bytes());
            matches += 1;
        } else if text.contains(LANG_NEW) {
            matches += 1;
        }
    }
    ensure!(matches > 0, "本地界面的语言协商格式已变化");

    for (relative, bytes) in catalogs {
        contents.insert(format!("i18n/{relative}"), bytes.to_vec());
    }
    for path in contents.keys() {
        entry_mut(&mut header, path)?;
    }

    let mut langs: Vec<String> = header["files"]["i18n"]["files"]
        .as_object()
        .map(|files| {
            files
                .iter()
                .filter(|(_, child)| child.get("files").is_none())
                .filter_map(|(name, _)| name.strip_suffix(".json"))
                .filter(|stem| !stem.contains('.'))
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    langs.sort();
    let index_meta = entry(&header, "index.html").context("ion-dist.asar 里缺少 index.html")?;
    let html = std::str::from_utf8(asar.blob(index_meta)?).context("index.html 不是 UTF-8")?;
    contents.insert("index.html".to_string(), register_catalogs(html, &langs)?.into_bytes());

    // 按新的遍历顺序重排 offset；未改动的文件沿用原内容与 integrity。
    let mut ordered = Vec::new();
    file_paths(&header, "", &mut ordered);
    let mut plan = Vec::with_capacity(ordered.len());
    let mut offset = 0usize;
    for path in &ordered {
        let content = match contents.remove(path) {
            Some(bytes) => Content::Replaced(bytes),
            None => {
                let meta = entry(&asar.header, path).context("ion-dist.asar 条目缺失")?;
                let blob = asar.blob(meta)?;
                Content::Original { offset: asar.data_start + meta["offset"].as_str().unwrap().parse::<usize>()?, size: blob.len() }
            }
        };
        let node = entry_mut(&mut header, path)?.as_object_mut().context("ion-dist.asar 头部结构异常")?;
        let size = match &content {
            Content::Replaced(bytes) => {
                node.insert("integrity".into(), integrity(bytes));
                bytes.len()
            }
            Content::Original { size, .. } => *size,
        };
        node.insert("size".into(), json!(size));
        node.insert("offset".into(), json!(offset.to_string()));
        offset += size;
        plan.push(content);
    }

    let text = serde_json::to_vec(&header)?;
    let pad = (4 - text.len() % 4) % 4;
    let json_size = text.len() + pad + 4;
    let mut out = Vec::with_capacity(16 + json_size + offset);
    for word in [4usize, json_size + 4, json_size, text.len()] {
        out.extend_from_slice(&(word as u32).to_le_bytes());
    }
    out.extend_from_slice(&text);
    out.resize(out.len() + pad, 0);
    for content in plan {
        match content {
            Content::Replaced(bytes) => out.extend_from_slice(&bytes),
            Content::Original { offset, size } => out.extend_from_slice(&asar.bytes[offset..offset + size]),
        }
    }
    let temp = path.with_extension("asar.codex-plus-tmp");
    std::fs::write(&temp, &out).with_context(|| format!("无法写入 {}", temp.display()))?;
    std::fs::rename(&temp, path).with_context(|| format!("无法替换 {}", path.display()))?;
    Ok(())
}

/// 校验副本里的 `ion-dist.asar` 已装好中文词表并登记。
pub(crate) fn verify_ion_asar(path: &Path) -> Result<()> {
    let asar = Asar::parse(std::fs::read(path).with_context(|| format!("无法读取 {}", path.display()))?)?;
    for required in ["i18n/zh-Hans.json", "i18n/dynamic/zh-Hans.json"] {
        if entry(&asar.header, required).is_none() {
            bail!("中文词表缺失");
        }
    }
    let html = asar.blob(entry(&asar.header, "index.html").context("ion-dist.asar 里缺少 index.html")?)?;
    ensure!(String::from_utf8_lossy(html).contains("zh-Hans:"), "本地界面未注册中文词表");
    Ok(())
}

/// 测试用：按给定文件拼一个标准 asar。
#[cfg(test)]
pub(crate) fn build_test_asar(files: &[(&str, &[u8])]) -> Vec<u8> {
    let mut header = json!({ "files": {} });
    let mut data = Vec::new();
    for (path, bytes) in files {
        let node = entry_mut(&mut header, path).unwrap().as_object_mut().unwrap();
        node.insert("size".into(), json!(bytes.len()));
        node.insert("offset".into(), json!(data.len().to_string()));
        node.insert("integrity".into(), integrity(bytes));
        data.extend_from_slice(bytes);
    }
    let text = serde_json::to_vec(&header).unwrap();
    let pad = (4 - text.len() % 4) % 4;
    let json_size = text.len() + pad + 4;
    let mut out = Vec::new();
    for word in [4usize, json_size + 4, json_size, text.len()] {
        out.extend_from_slice(&(word as u32).to_le_bytes());
    }
    out.extend_from_slice(&text);
    out.resize(out.len() + pad, 0);
    out.extend_from_slice(&data);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(path: &Path, name: &str) -> (Value, Vec<u8>) {
        let asar = Asar::parse(std::fs::read(path).unwrap()).unwrap();
        let meta = entry(&asar.header, name).unwrap().clone();
        let blob = asar.blob(&meta).unwrap().to_vec();
        (meta, blob)
    }

    const CATALOGS: &[(&str, &[u8])] = &[
        ("zh-Hans.json", b"{\"hello\":\"\xe4\xbd\xa0\xe5\xa5\xbd\"}"),
        ("dynamic/zh-Hans.json", b"{}"),
        ("zh-Hans.overrides.json", b"{}"),
    ];

    #[test]
    fn localizes_packed_ion_dist_and_keeps_untouched_integrity() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ion-dist.asar");
        let js = format!("const langs={LANG_OLD};");
        std::fs::write(&path, build_test_asar(&[
            ("assets/app.js", js.as_bytes()),
            ("assets/other.js", b"console.log(1)"),
            ("i18n/en-US.json", b"{}"),
            ("i18n/dynamic/en-US.json", b"{}"),
            ("index.html", b"<html><head><meta name=\"i18n-catalogs\" data-i18n-catalogs=\"en-US:1.1.1\"></head></html>"),
        ])).unwrap();
        let (other_before, _) = read(&path, "assets/other.js");

        localize_ion_asar(&path, CATALOGS).unwrap();

        let (_, app) = read(&path, "assets/app.js");
        assert!(String::from_utf8(app).unwrap().contains(LANG_NEW));
        let (other_after, other) = read(&path, "assets/other.js");
        assert_eq!(other, b"console.log(1)");
        assert_eq!(other_after["integrity"], other_before["integrity"]);
        let (zh_meta, zh) = read(&path, "i18n/zh-Hans.json");
        assert_eq!(zh, CATALOGS[0].1);
        assert_eq!(zh_meta["integrity"], integrity(CATALOGS[0].1));
        let (_, html) = read(&path, "index.html");
        let html = String::from_utf8(html).unwrap();
        assert!(html.contains(r#"data-i18n-catalogs="en-US:1.1.1,zh-Hans:1.1.1""#), "{html}");
        assert_eq!(html.matches("i18n-catalogs").count(), 2, "只保留一行 meta：{html}");
        verify_ion_asar(&path).unwrap();
        // 再跑一次是幂等的：白名单已含 zh-Hans 也算匹配。
        localize_ion_asar(&path, CATALOGS).unwrap();
        verify_ion_asar(&path).unwrap();
    }

    #[test]
    fn rejects_unknown_language_negotiation_and_leaves_file_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ion-dist.asar");
        let original = build_test_asar(&[("assets/app.js", b"const langs=[];"), ("index.html", b"<head></head>")]);
        std::fs::write(&path, &original).unwrap();
        let error = localize_ion_asar(&path, CATALOGS).unwrap_err();
        assert!(error.to_string().contains("语言协商"), "{error}");
        assert_eq!(std::fs::read(&path).unwrap(), original);
        assert!(verify_ion_asar(&path).is_err());
    }

    /// 本机装了新版 Claude 时，对它的 ion-dist.asar 的一份只读副本跑一遍。
    #[test]
    #[ignore]
    fn localizes_installed_claude_ion_asar_copy() {
        let source = Path::new("/Applications/Claude.app/Contents/Resources/ion-dist.asar");
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ion-dist.asar");
        std::fs::copy(source, &path).unwrap();
        let started = std::time::Instant::now();
        localize_ion_asar(&path, CATALOGS).unwrap();
        verify_ion_asar(&path).unwrap();
        eprintln!("localized in {:?}", started.elapsed());
    }
}
