//! 通用 Agent 的配置文件读写：JSON（容忍注释与尾逗号）、TOML、YAML（按顶层键整块改）、`.env`。
//!
//! 写配置分两步：各 Agent 的写入函数在 [`FileSet`] 上改内存里的文档，框架再统一落盘，
//! 同时记下每个文件接入前的原文和这次改到的键（见 [`crate::gateway::restore`]），断开时只把这些键改回去。
//!
//! YAML 没有可用的解析库：只支持「顶层键」这一级，`set` 整块替换一个顶层键（值由 [`yaml_block`] 生成），
//! 其余行（含注释）原样保留。各 Agent 的 YAML 写入都落在自己独占的顶层键上，或整块重写那一个键。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

use crate::gateway::restore::{self as stash, ChangedKey, FileStash, KeyPath};

/// 配置文件的格式。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FileFormat {
    Json,
    Toml,
    Yaml,
    Env,
    /// 整个文件由网关生成（例如单独的 `models.json`），还原时整份写回原文。
    Raw,
}

enum Doc {
    Json(Value),
    Toml(toml_edit::DocumentMut),
    Yaml(YamlDoc),
    Env(EnvDoc),
    Raw(Option<String>),
}

struct Entry {
    format: FileFormat,
    before: Option<String>,
    doc: Doc,
}

/// 一次接入要改的全部文件。
#[derive(Default)]
pub struct FileSet {
    entries: BTreeMap<PathBuf, Entry>,
}

/// 一个改动过的文件：接入前 / 后的全文。
#[derive(Debug, Clone)]
pub struct FileChange {
    pub path: PathBuf,
    pub format: FileFormat,
    pub before: Option<String>,
    pub after: Option<String>,
}

impl FileSet {
    pub fn new() -> Self {
        Self::default()
    }

    fn entry(&mut self, path: &Path, format: FileFormat) -> anyhow::Result<&mut Entry> {
        if !self.entries.contains_key(path) {
            let before = stash::read_optional(path)?;
            let doc = parse(format, before.as_deref())
                .with_context(|| format!("无法解析 {}，为免覆盖没有改动它", path.display()))?;
            self.entries.insert(
                path.to_path_buf(),
                Entry {
                    format,
                    before,
                    doc,
                },
            );
        }
        let entry = self.entries.get_mut(path).expect("刚插入");
        if entry.format != format {
            bail!("{} 被按两种格式打开", path.display());
        }
        Ok(entry)
    }

    /// JSON 文件的根对象（文件不存在时是空对象；根不是对象时换成对象）。
    pub fn json(&mut self, path: &Path) -> anyhow::Result<&mut Map<String, Value>> {
        let entry = self.entry(path, FileFormat::Json)?;
        let Doc::Json(value) = &mut entry.doc else {
            unreachable!()
        };
        if !value.is_object() {
            *value = Value::Object(Map::new());
        }
        Ok(value.as_object_mut().expect("刚换成对象"))
    }

    pub fn toml(&mut self, path: &Path) -> anyhow::Result<&mut toml_edit::DocumentMut> {
        let entry = self.entry(path, FileFormat::Toml)?;
        let Doc::Toml(doc) = &mut entry.doc else {
            unreachable!()
        };
        Ok(doc)
    }

    pub fn yaml(&mut self, path: &Path) -> anyhow::Result<&mut YamlDoc> {
        let entry = self.entry(path, FileFormat::Yaml)?;
        let Doc::Yaml(doc) = &mut entry.doc else {
            unreachable!()
        };
        Ok(doc)
    }

    pub fn env(&mut self, path: &Path) -> anyhow::Result<&mut EnvDoc> {
        let entry = self.entry(path, FileFormat::Env)?;
        let Doc::Env(doc) = &mut entry.doc else {
            unreachable!()
        };
        Ok(doc)
    }

    /// 整份由网关生成的文件；`None` 表示删掉。
    pub fn raw(&mut self, path: &Path) -> anyhow::Result<&mut Option<String>> {
        let entry = self.entry(path, FileFormat::Raw)?;
        let Doc::Raw(text) = &mut entry.doc else {
            unreachable!()
        };
        Ok(text)
    }

    /// 内容有变化的文件。
    pub fn changes(&self) -> anyhow::Result<Vec<FileChange>> {
        let mut out = Vec::new();
        for (path, entry) in &self.entries {
            let after = render(&entry.doc, entry.before.as_deref())?;
            let same = match (&entry.before, &after) {
                (None, None) => true,
                (Some(before), Some(after)) => {
                    before == after
                        || content_value(entry.format, Some(before))?
                            == content_value(entry.format, Some(after))?
                }
                _ => false,
            };
            if same {
                continue;
            }
            out.push(FileChange {
                path: path.clone(),
                format: entry.format,
                before: entry.before.clone(),
                after,
            });
        }
        Ok(out)
    }
}

fn parse(format: FileFormat, text: Option<&str>) -> anyhow::Result<Doc> {
    Ok(match format {
        FileFormat::Json => Doc::Json(parse_jsonc(text.unwrap_or_default())?),
        FileFormat::Toml => Doc::Toml(
            text.unwrap_or_default()
                .parse::<toml_edit::DocumentMut>()
                .map_err(|error| {
                    stash::toml_syntax_error("配置", text.unwrap_or_default(), error.span())
                })?,
        ),
        FileFormat::Yaml => Doc::Yaml(YamlDoc::parse(text.unwrap_or_default())),
        FileFormat::Env => Doc::Env(EnvDoc::parse(text.unwrap_or_default())),
        FileFormat::Raw => Doc::Raw(text.map(str::to_string)),
    })
}

fn render(doc: &Doc, before: Option<&str>) -> anyhow::Result<Option<String>> {
    Ok(match doc {
        Doc::Json(value) => Some(super::jsonc_edit::render(before, value)?),
        Doc::Toml(doc) => Some(doc.to_string()),
        Doc::Yaml(doc) => Some(doc.render()),
        Doc::Env(doc) => Some(doc.render()),
        Doc::Raw(text) => text.clone(),
    })
}

/// 比较、记录改动用的内容（与格式无关的 JSON 值）。
pub fn content_value(format: FileFormat, text: Option<&str>) -> anyhow::Result<Value> {
    Ok(match format {
        FileFormat::Json => parse_jsonc(text.unwrap_or_default())?,
        FileFormat::Toml => stash::toml_value(text)?,
        FileFormat::Yaml => YamlDoc::parse(text.unwrap_or_default()).as_value(),
        FileFormat::Env => EnvDoc::parse(text.unwrap_or_default()).as_value(),
        FileFormat::Raw => Value::String(text.unwrap_or_default().to_string()),
    })
}

// ---------------------------------------------------------------- JSON（容忍注释）

/// 解析 JSON，容忍 `//`、`/* */` 注释和尾逗号（VS Code、Zed、OpenCode 的配置都允许）。空文本是空对象。
pub fn parse_jsonc(text: &str) -> anyhow::Result<Value> {
    let cleaned = strip_jsonc(text);
    if cleaned.trim().is_empty() {
        return Ok(Value::Object(Map::new()));
    }
    serde_json::from_str(&cleaned).context("不是合法的 JSON")
}

/// 去掉注释与尾逗号，字符串里的内容不动。
pub fn strip_jsonc(text: &str) -> String {
    let chars: Vec<char> = text.trim_start_matches('\u{feff}').chars().collect();
    let mut out = String::with_capacity(chars.len());
    let mut index = 0;
    let mut in_string = false;
    while index < chars.len() {
        let ch = chars[index];
        if in_string {
            out.push(ch);
            if ch == '\\' {
                if let Some(next) = chars.get(index + 1) {
                    out.push(*next);
                    index += 1;
                }
            } else if ch == '"' {
                in_string = false;
            }
            index += 1;
            continue;
        }
        match ch {
            '"' => {
                in_string = true;
                out.push(ch);
                index += 1;
            }
            '/' if chars.get(index + 1) == Some(&'/') => {
                while index < chars.len() && chars[index] != '\n' {
                    index += 1;
                }
            }
            '/' if chars.get(index + 1) == Some(&'*') => {
                index += 2;
                while index < chars.len()
                    && !(chars[index] == '*' && chars.get(index + 1) == Some(&'/'))
                {
                    index += 1;
                }
                index += 2;
            }
            ',' => {
                // 尾逗号：后面（跳过空白和注释）紧跟 `}` 或 `]` 时丢掉。
                let mut look = index + 1;
                loop {
                    while look < chars.len() && chars[look].is_whitespace() {
                        look += 1;
                    }
                    if chars.get(look) == Some(&'/') && chars.get(look + 1) == Some(&'/') {
                        while look < chars.len() && chars[look] != '\n' {
                            look += 1;
                        }
                        continue;
                    }
                    if chars.get(look) == Some(&'/') && chars.get(look + 1) == Some(&'*') {
                        look += 2;
                        while look < chars.len()
                            && !(chars[look] == '*' && chars.get(look + 1) == Some(&'/'))
                        {
                            look += 1;
                        }
                        look += 2;
                        continue;
                    }
                    break;
                }
                if !matches!(chars.get(look), Some('}') | Some(']')) {
                    out.push(ch);
                }
                index += 1;
            }
            _ => {
                out.push(ch);
                index += 1;
            }
        }
    }
    out
}

/// 取对象里的子对象，没有（或不是对象）时建一个空的。
pub fn object_entry<'a>(map: &'a mut Map<String, Value>, key: &str) -> &'a mut Map<String, Value> {
    let slot = map
        .entry(key.to_string())
        .or_insert_with(|| Value::Object(Map::new()));
    if !slot.is_object() {
        *slot = Value::Object(Map::new());
    }
    slot.as_object_mut().expect("刚换成对象")
}

// ---------------------------------------------------------------- YAML（顶层键）

/// YAML 文档：按顶层键切块，块内原文保留。
#[derive(Debug, Clone, Default)]
pub struct YamlDoc {
    /// 第一个顶层键之前的内容（注释、`---`）。
    head: String,
    blocks: Vec<(String, String)>,
}

impl YamlDoc {
    pub fn parse(text: &str) -> Self {
        let mut doc = YamlDoc::default();
        for line in text.split_inclusive('\n') {
            if let Some(key) = top_level_key(line) {
                doc.blocks.push((key, line.to_string()));
                continue;
            }
            match doc.blocks.last_mut() {
                Some((_, block)) => block.push_str(line),
                None => doc.head.push_str(line),
            }
        }
        doc
    }

    pub fn render(&self) -> String {
        let mut out = self.head.clone();
        for (_, block) in &self.blocks {
            if !out.is_empty() && !out.ends_with('\n') {
                out.push('\n');
            }
            out.push_str(block);
        }
        out
    }

    pub fn has(&self, key: &str) -> bool {
        self.blocks.iter().any(|(name, _)| name == key)
    }

    /// 顶层键的原文块（含键名那一行）。
    pub fn block(&self, key: &str) -> Option<&str> {
        self.blocks
            .iter()
            .find(|(name, _)| name == key)
            .map(|(_, block)| block.as_str())
    }

    /// 顶层标量键的值（去掉引号）；不是标量或没有时为 None。
    pub fn scalar(&self, key: &str) -> Option<String> {
        let block = self.block(key)?;
        let first = block.lines().next()?;
        let value = first.split_once(':')?.1.trim();
        let value = strip_yaml_comment(value).trim();
        if value.is_empty() {
            return None;
        }
        Some(unquote_yaml(value))
    }

    /// 整块写入一个顶层键；原来有就原地替换，没有加在末尾。
    pub fn set(&mut self, key: &str, value: &Value) {
        self.set_block(key, yaml_block(key, value));
    }

    /// 用现成的原文块写入（还原时用）。
    pub fn set_block(&mut self, key: &str, mut block: String) {
        if !block.ends_with('\n') {
            block.push('\n');
        }
        match self.blocks.iter_mut().find(|(name, _)| name == key) {
            Some((_, existing)) => {
                // 保留原块尾部的空行与注释（属于下一段的说明）。
                let trailing = trailing_comments(existing);
                *existing = block + &trailing;
            }
            None => self.blocks.push((key.to_string(), block)),
        }
    }

    pub fn remove(&mut self, key: &str) {
        self.blocks.retain(|(name, _)| name != key);
    }

    /// 比较用：每个顶层键的原文（去掉注释和空行）。
    fn as_value(&self) -> Value {
        let mut map = Map::new();
        for (key, block) in &self.blocks {
            let normalized = block
                .lines()
                .filter(|line| {
                    let trimmed = line.trim();
                    !trimmed.is_empty() && !trimmed.starts_with('#')
                })
                .map(str::trim_end)
                .collect::<Vec<_>>()
                .join("\n");
            map.insert(key.clone(), Value::String(normalized));
        }
        Value::Object(map)
    }
}

fn top_level_key(line: &str) -> Option<String> {
    let first = line.chars().next()?;
    if first.is_whitespace() || matches!(first, '#' | '-' | '}' | ']' | '.') {
        return None;
    }
    let (key, _) = line.split_once(':')?;
    let key = key.trim();
    if key.is_empty() {
        return None;
    }
    Some(unquote_yaml(key))
}

fn trailing_comments(block: &str) -> String {
    let lines: Vec<&str> = block.split_inclusive('\n').collect();
    let mut start = lines.len();
    while start > 1 {
        let trimmed = lines[start - 1].trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            start -= 1;
        } else {
            break;
        }
    }
    lines[start..].concat()
}

fn strip_yaml_comment(value: &str) -> &str {
    if value.starts_with('"') || value.starts_with('\'') {
        return value;
    }
    match value.find(" #") {
        Some(index) => &value[..index],
        None => value,
    }
}

fn unquote_yaml(value: &str) -> String {
    let value = value.trim();
    if value.len() >= 2
        && ((value.starts_with('"') && value.ends_with('"'))
            || (value.starts_with('\'') && value.ends_with('\'')))
    {
        return value[1..value.len() - 1].to_string();
    }
    value.to_string()
}

/// 把一个顶层键和它的值写成 YAML 块。
pub fn yaml_block(key: &str, value: &Value) -> String {
    let mut out = String::new();
    write_yaml_entry(&mut out, 0, key, value);
    out
}

fn write_yaml_entry(out: &mut String, indent: usize, key: &str, value: &Value) {
    let pad = " ".repeat(indent);
    let key = yaml_key(key);
    match value {
        Value::Object(map) if !map.is_empty() => {
            out.push_str(&format!("{pad}{key}:\n"));
            for (child, value) in map {
                write_yaml_entry(out, indent + 2, child, value);
            }
        }
        Value::Array(items) if !items.is_empty() => {
            out.push_str(&format!("{pad}{key}:\n"));
            for item in items {
                write_yaml_item(out, indent + 2, item);
            }
        }
        _ => out.push_str(&format!("{pad}{key}: {}\n", yaml_scalar(value))),
    }
}

fn write_yaml_item(out: &mut String, indent: usize, value: &Value) {
    let pad = " ".repeat(indent);
    match value {
        Value::Object(map) if !map.is_empty() => {
            let mut first = true;
            for (key, value) in map {
                let mut entry = String::new();
                write_yaml_entry(&mut entry, indent + 2, key, value);
                if first {
                    // 第一项挂在 `- ` 后面。
                    out.push_str(&format!("{pad}- {}", &entry[indent + 2..]));
                    first = false;
                } else {
                    out.push_str(&entry);
                }
            }
        }
        _ => out.push_str(&format!("{pad}- {}\n", yaml_scalar(value))),
    }
}

fn yaml_key(key: &str) -> String {
    let plain = !key.is_empty()
        && key
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.' | '/'));
    if plain {
        key.to_string()
    } else {
        serde_json::to_string(key).unwrap_or_default()
    }
}

fn yaml_scalar(value: &Value) -> String {
    match value {
        Value::Null => "null".to_string(),
        Value::Bool(flag) => flag.to_string(),
        Value::Number(number) => number.to_string(),
        // JSON 字符串字面量也是合法的 YAML 双引号字符串。
        Value::String(text) => serde_json::to_string(text).unwrap_or_default(),
        Value::Array(_) => "[]".to_string(),
        Value::Object(_) => "{}".to_string(),
    }
}

// ---------------------------------------------------------------- .env

/// `.env` 文件：`KEY=VALUE` 行，其余行原样保留。
#[derive(Debug, Clone, Default)]
pub struct EnvDoc {
    lines: Vec<String>,
}

impl EnvDoc {
    pub fn parse(text: &str) -> Self {
        Self {
            lines: text.lines().map(str::to_string).collect(),
        }
    }

    pub fn render(&self) -> String {
        if self.lines.is_empty() {
            return String::new();
        }
        let mut out = self.lines.join("\n");
        out.push('\n');
        out
    }

    fn position(&self, key: &str) -> Option<usize> {
        self.lines
            .iter()
            .position(|line| env_line_key(line).as_deref() == Some(key))
    }

    pub fn get(&self, key: &str) -> Option<String> {
        let line = &self.lines[self.position(key)?];
        let (_, value) = line.split_once('=')?;
        Some(unquote_yaml(value.trim()))
    }

    pub fn set(&mut self, key: &str, value: &str) {
        let line = format!("{key}={value}");
        match self.position(key) {
            Some(index) => self.lines[index] = line,
            None => self.lines.push(line),
        }
    }

    /// 写回原来的整行（还原时用）。
    fn set_line(&mut self, key: &str, line: String) {
        match self.position(key) {
            Some(index) => self.lines[index] = line,
            None => self.lines.push(line),
        }
    }

    fn line(&self, key: &str) -> Option<&String> {
        self.position(key).map(|index| &self.lines[index])
    }

    pub fn remove(&mut self, key: &str) {
        self.lines
            .retain(|line| env_line_key(line).as_deref() != Some(key));
    }

    fn as_value(&self) -> Value {
        let mut map = Map::new();
        for line in &self.lines {
            if let Some(key) = env_line_key(line) {
                map.insert(key, Value::String(line.trim().to_string()));
            }
        }
        Value::Object(map)
    }
}

fn env_line_key(line: &str) -> Option<String> {
    let trimmed = line.trim_start();
    if trimmed.starts_with('#') {
        return None;
    }
    let trimmed = trimmed.strip_prefix("export ").unwrap_or(trimmed);
    let (key, _) = trimmed.split_once('=')?;
    let key = key.trim();
    (!key.is_empty() && !key.contains(char::is_whitespace)).then(|| key.to_string())
}

// ---------------------------------------------------------------- 存档与还原

/// 一个文件的存档：格式 + 接入前原文 + 网关改过的键。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StashedFile {
    pub format: FileFormat,
    #[serde(flatten)]
    pub stash: FileStash,
    /// 最后一次网关写入的字节指纹；只有原文未被用户改动时才可整份还原旧格式。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub written_hash: Option<String>,
    /// 接入期间观察到外部编辑后，后续网关刷新也不能让整份旧格式覆盖它。
    #[serde(default)]
    pub preserve_current_format: bool,
}

/// 一个通用 Agent 接入前的存档：每个改过的文件一份。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentStash {
    #[serde(default)]
    pub files: BTreeMap<PathBuf, StashedFile>,
}

impl AgentStash {
    /// 记下一次写入：第一次碰到的文件记接入前的原文，之后只追加改到的键。
    pub fn record(&mut self, change: &FileChange) -> anyhow::Result<()> {
        let before = content_value(change.format, change.before.as_deref())?;
        let after = content_value(change.format, change.after.as_deref())?;
        let existed = self.files.contains_key(&change.path);
        let file = self
            .files
            .entry(change.path.clone())
            .or_insert_with(|| StashedFile {
                format: change.format,
                stash: FileStash::new(change.before.clone()),
                written_hash: None,
                preserve_current_format: false,
            });
        if change.format == FileFormat::Json {
            if existed
                && !file.written_hash.as_ref().is_some_and(|written| {
                    change.before.as_deref().is_some_and(|text| text_hash(text) == *written)
                })
            {
                file.preserve_current_format = true;
            }
            file.written_hash = change.after.as_deref().map(text_hash);
        }
        if change.format == FileFormat::Raw {
            // 整份文件：记一个空路径，还原时整份写回。
            if file.stash.changed.is_empty() {
                file.stash.changed.push(ChangedKey {
                    path: Vec::new(),
                    written: None,
                });
            }
            return Ok(());
        }
        if matches!(change.format, FileFormat::Yaml | FileFormat::Env) {
            // 只到顶层键这一级。
            for path in stash::changed_paths(&before, &after) {
                let top: KeyPath = path.into_iter().take(1).collect();
                if !file.stash.changed.iter().any(|known| known.path == top) {
                    file.stash.changed.push(ChangedKey {
                        path: top,
                        written: None,
                    });
                }
            }
            return Ok(());
        }
        file.stash.record_write(&before, &after);
        Ok(())
    }
}

/// 把一个文件改回接入前：只还原网关改过的键，接入期间别的改动保留；
/// 与接入前内容相同时写回原文。返回要写入的内容（`None` 表示删掉文件）。
pub fn restored_text(
    file: &StashedFile,
    current: Option<&str>,
) -> anyhow::Result<Option<String>> {
    let before = file.stash.before.as_deref();
    match file.format {
        FileFormat::Raw => Ok(before.map(str::to_string)),
        FileFormat::Json => {
            let current_clean = current.map(strip_jsonc);
            let before_clean = before.map(strip_jsonc);
            let restored = stash::restore_json(
                before_clean.as_deref(),
                current_clean.as_deref(),
                &file.stash.changed,
                |_, _, _| false,
            )?;
            // 语义相同不足以证明用户没改注释或格式，只有字节指纹匹配才整份写回。
            if restored.as_deref() == before_clean.as_deref()
                && !file.preserve_current_format
                && file.written_hash.as_ref().is_some_and(|written| {
                    current.is_some_and(|text| text_hash(text) == *written)
                })
            {
                return Ok(before.map(str::to_string));
            }
            let desired = parse_jsonc(restored.as_deref().unwrap_or("{}"))?;
            super::jsonc_edit::render(current, &desired).map(Some)
        }
        FileFormat::Toml => stash::restore_toml(before, current, &file.stash.paths()),
        FileFormat::Yaml => {
            let before_doc = YamlDoc::parse(before.unwrap_or_default());
            let mut doc = YamlDoc::parse(current.unwrap_or_default());
            for path in file.stash.paths() {
                let Some(key) = path.first() else { continue };
                match before_doc.block(key) {
                    Some(block) => doc.set_block(key, block.to_string()),
                    None => doc.remove(key),
                }
            }
            if doc.as_value() == before_doc.as_value() {
                return Ok(before.map(str::to_string));
            }
            Ok(Some(doc.render()))
        }
        FileFormat::Env => {
            let before_doc = EnvDoc::parse(before.unwrap_or_default());
            let mut doc = EnvDoc::parse(current.unwrap_or_default());
            for path in file.stash.paths() {
                let Some(key) = path.first() else { continue };
                match before_doc.line(key) {
                    Some(line) => doc.set_line(key, line.clone()),
                    None => doc.remove(key),
                }
            }
            if doc.as_value() == before_doc.as_value() {
                return Ok(before.map(str::to_string));
            }
            Ok(Some(doc.render()))
        }
    }
}

fn text_hash(text: &str) -> String {
    format!("{:x}", Sha256::digest(text.as_bytes()))
}

/// 写入（必要时建目录）；`None` 删掉文件。
pub fn write_file(path: &Path, contents: Option<&str>) -> anyhow::Result<()> {
    if contents.is_some()
        && let Some(parent) = path.parent()
    {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("无法创建目录 {}", parent.display()))?;
    }
    stash::write_or_remove(path, contents)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn jsonc_comments_and_trailing_commas() {
        let text = r#"{
  // 注释
  "a": "http://x//y", /* 块 */
  "b": [1, 2,],
}"#;
        assert_eq!(
            parse_jsonc(text).unwrap(),
            json!({ "a": "http://x//y", "b": [1, 2] })
        );
    }

    #[test]
    fn yaml_set_replaces_only_that_block() {
        let text = "# 头\nmodel:\n  default: gpt\n  provider: auto\n\n# 下一段\nother: 1\n";
        let mut doc = YamlDoc::parse(text);
        doc.set(
            "model",
            &json!({ "default": "g/m", "provider": "custom", "base_url": "http://h" }),
        );
        let out = doc.render();
        assert!(out.starts_with("# 头\nmodel:\n"));
        assert!(out.contains("  provider: \"custom\"\n"));
        assert!(out.contains("\n# 下一段\nother: 1\n"));
        assert_eq!(doc.scalar("other").as_deref(), Some("1"));
    }

    #[test]
    fn yaml_list_of_objects() {
        let block = yaml_block("providers", &json!([{ "name": "a", "url": "u" }, "x"]));
        assert_eq!(
            block,
            "providers:\n  - name: \"a\"\n    url: \"u\"\n  - \"x\"\n"
        );
    }

    #[test]
    fn env_set_and_restore() {
        let before = "# keep\nA=1\nB=2\n";
        let mut doc = EnvDoc::parse(before);
        doc.set("B", "9");
        doc.set("C", "3");
        let change = FileChange {
            path: PathBuf::from("/x/.env"),
            format: FileFormat::Env,
            before: Some(before.to_string()),
            after: Some(doc.render()),
        };
        let mut agent_stash = AgentStash::default();
        agent_stash.record(&change).unwrap();
        let file = agent_stash.files.values().next().unwrap();
        // 接入期间用户加了一行 D，还原后保留。
        let current = format!("{}D=4\n", doc.render());
        let restored = restored_text(file, Some(&current)).unwrap().unwrap();
        assert_eq!(restored, "# keep\nA=1\nB=2\nD=4\n");
        assert_eq!(
            restored_text(file, change.after.as_deref()).unwrap().as_deref(),
            Some(before)
        );
    }

    #[test]
    fn json_restore_keeps_comments_when_unchanged() {
        let before = "{\n  // 我的注释\n  \"a\": 1\n}\n";
        let mut value = parse_jsonc(before).unwrap();
        value["gw"] = json!({ "url": "http://127.0.0.1/gw/x" });
        let after = serde_json::to_string_pretty(&value).unwrap();
        let change = FileChange {
            path: PathBuf::from("/x/a.json"),
            format: FileFormat::Json,
            before: Some(before.to_string()),
            after: Some(after.clone()),
        };
        let mut agent_stash = AgentStash::default();
        agent_stash.record(&change).unwrap();
        let file = agent_stash.files.values().next().unwrap();
        assert_eq!(
            restored_text(file, Some(&after)).unwrap().as_deref(),
            Some(before)
        );
    }

    #[test]
    fn new_file_restores_to_absent() {
        let change = FileChange {
            path: PathBuf::from("/x/new.json"),
            format: FileFormat::Json,
            before: None,
            after: Some("{\"gw\":1}".to_string()),
        };
        let mut agent_stash = AgentStash::default();
        agent_stash.record(&change).unwrap();
        let file = agent_stash.files.values().next().unwrap();
        assert_eq!(restored_text(file, change.after.as_deref()).unwrap(), None);
    }
}
