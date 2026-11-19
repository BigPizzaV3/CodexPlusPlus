//! Codex 自定义 prompt（`<CODEX_HOME>/prompts/*.md`）的只读清单。
//!
//! Codex 应用自己会在页面加载时读同一个目录，把每个文件做成 `/prompts:<文件名>` 斜杠命令，
//! 但发送时只是「正文 + 空格 + 参数」，不认 CLI 的占位符（`$1`…`$9`、`$ARGUMENTS`、`$名字`）。
//! 注入页（`96-prompt-args.js`）用这份清单在发送前按 CLI 规则展开，解析规则与应用保持一致：
//! 只读顶层的 `.md` 普通文件，可选的 `---` frontmatter 里只认 `description` 与 `argument-hint`。

use std::path::Path;

use serde::Serialize;

/// 单个文件的上限；超过的不读（prompt 文件不该这么大，避免误放的大文件拖慢页面）。
const MAX_PROMPT_BYTES: u64 = 256 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CustomPrompt {
    pub id: String,
    pub description: Option<String>,
    pub argument_hint: Option<String>,
    pub content: String,
}

/// 读 `<codex_home>/prompts` 下的 prompt，按 id 排序。目录不存在时返回空清单。
pub fn list_custom_prompts(codex_home: &Path) -> Vec<CustomPrompt> {
    let dir = codex_home.join("prompts");
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut prompts = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(id) = path
            .extension()
            .filter(|ext| *ext == "md")
            .and_then(|_| path.file_stem())
            .and_then(|stem| stem.to_str())
            .map(str::to_owned)
        else {
            continue;
        };
        // 和应用一样只认普通文件（Dirent.isFile 不跟随符号链接）。
        let Ok(file_type) = entry.file_type() else { continue };
        if !file_type.is_file() {
            continue;
        }
        if entry.metadata().map(|meta| meta.len() > MAX_PROMPT_BYTES).unwrap_or(true) {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else { continue };
        prompts.push(parse_custom_prompt(id, &text));
    }
    prompts.sort_by(|left, right| left.id.cmp(&right.id));
    prompts
}

/// 拆 frontmatter 与正文。对应应用里的 `/^---\s*\n([\s\S]*?)\n---\s*/`。
pub fn parse_custom_prompt(id: String, text: &str) -> CustomPrompt {
    let mut prompt = CustomPrompt { id, description: None, argument_hint: None, content: text.trim().to_owned() };
    let Some((front, body)) = split_frontmatter(text) else {
        return prompt;
    };
    for line in front.lines() {
        let Some((key, value)) = line.split_once(':') else { continue };
        let value = strip_quotes(value.trim()).to_owned();
        match key.trim() {
            "description" => prompt.description = Some(value),
            "argument-hint" => prompt.argument_hint = Some(value),
            _ => {}
        }
    }
    prompt.content = body.trim().to_owned();
    prompt
}

fn split_frontmatter(text: &str) -> Option<(&str, &str)> {
    let rest = text.strip_prefix("---")?;
    // `---` 后面的空白（含换行）都吃掉，但至少要有一个换行。
    let start = rest.len() - rest.trim_start().len();
    if !rest[..start].contains('\n') {
        return None;
    }
    let inner = &rest[start..];
    // 懒惰匹配：找第一个「换行 + ---」。
    let close = inner.find("\n---")?;
    let front = &inner[..close];
    let after = &inner[close + 4..];
    Some((front, after))
}

fn strip_quotes(value: &str) -> &str {
    for quote in ['"', '\''] {
        if value.len() >= 2 && value.starts_with(quote) && value.ends_with(quote) {
            return &value[1..value.len() - 1];
        }
    }
    value
}
