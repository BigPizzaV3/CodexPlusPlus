//! DeepSeek Harness 的接入：cordis 补丁层（YAML）。
//!
//! DSH 桌面版的 profile 由若干 bundle 叠加而成，用户可改的只有
//! `~/.dsh/profiles/desktop/cordis.patch.yml`——一个顶层 YAML 数组，每项是一条
//! **loader 补丁指令**，不是插件条目本身。MCP server 由
//! `@deepseek-ai/dsh-mcp-client` 插件承载，一个 server 插一条：
//!
//! ```yaml
//! - insert:
//!     - id: codex-plus-cu
//!       name: "@deepseek-ai/dsh-mcp-client"
//!       config:
//!         serverName: "codex-plus-cu"
//!         transport: stdio
//!         command: "/…/CodexPlusPlus"
//!         args: ["--codex-plus-cu-mcp"]
//! ```
//!
//! `insert` 不能省。`dsh-app-boot` 的 `applyEntryPatches` 对不带 `insert` 的项按
//! `id` 去已有条目里找目标：找不到就打一条 `patch: entry %C not found` 的警告然后跳过，
//! 启动不报错，插件也不会被加载——写错了完全没有可见反馈。
//!
//! 工具名会是 `mcp__codex-plus-cu__click`，与 Claude Code 一致。
//!
//! # 为什么是文本块而不是 YAML 解析器
//!
//! 补丁层里允许 `!!js` 表达式与用户注释，解析再序列化会把它们弄丢；仓库也没有
//! YAML 依赖。这里只在文件末尾维护一段带首尾标记的块，卸载时整段删除，用户写的
//! 其它内容一字不动。标量一律用 JSON 字符串写出（JSON 是 YAML 1.2 的子集），
//! 不必处理 YAML 的引号与转义规则。

use std::path::Path;

use anyhow::{Context, bail};
use serde_json::Value;

use super::RuntimeLocation;

/// 起始标记的固定前缀；后面的说明文字以后可以改，识别只看前缀。
const BEGIN_TAG: &str = "# >>> codex-plus-computer-use";
const BEGIN: &str = concat!(
    "# >>> codex-plus-computer-use",
    "：由 Codex++ 写入与维护，请在 Codex++ 的 Computer Use 页移除，不要手改"
);
const END: &str = "# <<< codex-plus-computer-use";
const PLUGIN: &str = "@deepseek-ai/dsh-mcp-client";

#[derive(Debug, PartialEq, Eq)]
pub(super) enum State {
    Missing,
    Current,
    Stale,
    Foreign,
}

pub(super) fn status(path: &Path, runtime: &RuntimeLocation) -> anyhow::Result<State> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(State::Missing),
        Err(error) => return Err(error).with_context(|| format!("读取 {} 失败", path.display())),
    };
    let (outside, block) = split_block(&text)?;
    if has_foreign_entry(&outside) {
        return Ok(State::Foreign);
    }
    Ok(match block {
        None => State::Missing,
        Some(block) if block == render(runtime) => State::Current,
        Some(_) => State::Stale,
    })
}

/// 返回写入后的全文；已是最新时返回 `None`。
pub(super) fn install(
    before: Option<&str>,
    runtime: &RuntimeLocation,
    path: &Path,
) -> anyhow::Result<Option<String>> {
    let text = before.unwrap_or("");
    let (outside, block) = split_block(text)?;
    if has_foreign_entry(&outside) {
        bail!(
            "{} 里已有 id 为 codex-plus-cu / codex-plus-browser 的条目且不是本功能写入的，未覆盖",
            path.display()
        );
    }
    let expected = render(runtime);
    if block.as_deref() == Some(expected.as_str()) {
        return Ok(None);
    }
    let base = strip_empty_flow_list(&outside, path)?;
    let base = base.trim_end();
    Ok(Some(if base.is_empty() {
        expected
    } else {
        format!("{base}\n\n{expected}")
    }))
}

/// 删掉托管块；没有托管块时返回 `None`。删空了就写回 `[]`，保持「顶层是数组」。
pub(super) fn uninstall(before: &str) -> Option<String> {
    let (outside, block) = split_block(before).ok()?;
    block?;
    let trimmed = outside.trim_end();
    Some(if content_lines(trimmed).next().is_none() {
        if trimmed.is_empty() {
            "[]\n".to_string()
        } else {
            format!("{trimmed}\n[]\n")
        }
    } else {
        format!("{trimmed}\n")
    })
}

fn render(runtime: &RuntimeLocation) -> String {
    let mut out = String::new();
    out.push_str(BEGIN);
    out.push('\n');
    // 一条 insert 补丁插入多个插件条目；拆成多条 insert 也行，合并更省行数。
    out.push_str("- insert:\n");
    for (id, entry) in runtime.managed_entries() {
        out.push_str(&format!("    - id: {id}\n"));
        out.push_str(&format!("      name: {}\n", json(&Value::from(PLUGIN))));
        out.push_str("      config:\n");
        out.push_str(&format!("        serverName: {}\n", json(&Value::from(id))));
        out.push_str("        transport: stdio\n");
        if let Some(command) = entry.get("command") {
            out.push_str(&format!("        command: {}\n", json(command)));
        }
        if let Some(args) = entry.get("args") {
            out.push_str(&format!("        args: {}\n", json(args)));
        }
        if let Some(env) = entry.get("env").and_then(Value::as_object) {
            out.push_str("        env:\n");
            for (key, value) in env {
                out.push_str(&format!("          {key}: {}\n", json(value)));
            }
        }
    }
    out.push_str(END);
    out.push('\n');
    out
}

fn json(value: &Value) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "null".into())
}

/// 把全文拆成「托管块以外的部分」与「托管块（含首尾标记）」。
fn split_block(text: &str) -> anyhow::Result<(String, Option<String>)> {
    let lines: Vec<&str> = text.lines().collect();
    let start = lines.iter().position(|line| line.starts_with(BEGIN_TAG));
    let Some(start) = start else {
        return Ok((text.to_string(), None));
    };
    let Some(offset) = lines[start..].iter().position(|line| line.trim_end() == END) else {
        bail!("补丁层里的 Codex++ 托管块缺少结束标记，未修改");
    };
    let end = start + offset;
    let mut block = lines[start..=end].join("\n");
    block.push('\n');
    let mut outside: Vec<&str> = lines[..start].to_vec();
    // 去掉托管块前面为了分隔而留的空行，避免反复安装卸载后空行越积越多。
    while outside.last().is_some_and(|line| line.trim().is_empty()) {
        outside.pop();
    }
    let rest: Vec<&str> = lines[end + 1..]
        .iter()
        .copied()
        .skip_while(|line| line.trim().is_empty())
        .collect();
    if !rest.is_empty() {
        if !outside.is_empty() {
            outside.push("");
        }
        outside.extend(rest);
    }
    let mut outside = outside.join("\n");
    if !outside.is_empty() {
        outside.push('\n');
    }
    Ok((outside, Some(block)))
}

/// 非空、非注释行。
fn content_lines(text: &str) -> impl Iterator<Item = &str> {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
}

/// 托管块要以块序列的形式追加在文件末尾，只认两种形状：空数组 `[]`（去掉它）
/// 与块序列（`- …` 开头）。其它形状（流式数组、映射）追加后会变成非法 YAML，拒绝修改。
fn strip_empty_flow_list(text: &str, path: &Path) -> anyhow::Result<String> {
    let first = content_lines(text).next();
    match first {
        None => Ok(text.to_string()),
        Some("[]") if content_lines(text).count() == 1 => Ok(text
            .lines()
            .filter(|line| line.trim() != "[]")
            .collect::<Vec<_>>()
            .join("\n")),
        Some(line) if line.starts_with('-') => Ok(text.to_string()),
        Some(_) => bail!(
            "{} 的顶层不是块序列（`- …`），无法安全追加，未修改",
            path.display()
        ),
    }
}

/// 托管块以外是否已经有同名条目（用户手写或其它工具写入）。
fn has_foreign_entry(outside: &str) -> bool {
    outside.lines().any(|line| {
        let line = line.trim().trim_start_matches('-').trim();
        let Some(value) = line
            .strip_prefix("id:")
            .or_else(|| line.strip_prefix("serverName:"))
        else {
            return false;
        };
        let value = value.trim().trim_matches(|c| c == '"' || c == '\'');
        value == super::SERVER_ID || value == super::BROWSER_SERVER_ID
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn runtime() -> RuntimeLocation {
        RuntimeLocation {
            app_path: "/Applications/ChatGPT.app".into(),
            node: "/x/node".into(),
            client: "/x/SkyComputerUseClient".into(),
            cua_repl: "/x/cua-repl.mjs".into(),
            node_repl: "/x/node_repl".into(),
            node_module_dirs: "/x/node_modules".into(),
            codex_home: "/x/codex".into(),
            service_path: None,
            codex_cli: None,
            app_version: None,
            package_version: None,
        }
    }

    const USER_PATCH: &str = "# Your patch layer\n- id: agent-default-model\n  name: \"@deepseek-ai/dsh-agent-default-model\"\n  config:\n    model: deepseek-flash\n- id: session-delete\n  disabled: false\n";

    #[test]
    fn install_appends_block_and_keeps_user_entries() {
        let path = Path::new("/tmp/cordis.patch.yml");
        let out = install(Some(USER_PATCH), &runtime(), path).unwrap().unwrap();
        assert!(out.starts_with(USER_PATCH.trim_end()));
        // 条目必须包在 insert 补丁里，否则 loader 按 id 找不到目标就静默跳过。
        assert!(out.contains("\n- insert:\n"));
        assert!(out.contains("    - id: codex-plus-cu\n"));
        assert!(out.contains("    - id: codex-plus-browser\n"));
        assert!(out.contains("      name: \"@deepseek-ai/dsh-mcp-client\"\n"));
        assert!(out.contains("        args: [\"--codex-plus-browser-mcp\"]\n"));
        assert!(out.ends_with(&format!("{END}\n")));
        // 再装一次不改。
        assert!(install(Some(&out), &runtime(), path).unwrap().is_none());
    }

    #[test]
    fn uninstall_restores_original_text() {
        let path = Path::new("/tmp/cordis.patch.yml");
        let installed = install(Some(USER_PATCH), &runtime(), path).unwrap().unwrap();
        assert_eq!(uninstall(&installed).unwrap(), USER_PATCH);
        assert!(uninstall(USER_PATCH).is_none());
    }

    #[test]
    fn empty_flow_list_is_replaced_and_restored() {
        let path = Path::new("/tmp/cordis.patch.yml");
        let original = "# header\n[]\n";
        let installed = install(Some(original), &runtime(), path).unwrap().unwrap();
        assert!(!installed.lines().any(|line| line.trim() == "[]"));
        assert!(installed.starts_with("# header\n"));
        assert_eq!(uninstall(&installed).unwrap(), original);
    }

    #[test]
    fn missing_file_gets_only_the_block() {
        let path = Path::new("/tmp/cordis.patch.yml");
        let installed = install(None, &runtime(), path).unwrap().unwrap();
        assert!(installed.starts_with(BEGIN));
        assert_eq!(uninstall(&installed).unwrap(), "[]\n");
    }

    #[test]
    fn foreign_entry_and_unsupported_shapes_are_refused() {
        let path = Path::new("/tmp/cordis.patch.yml");
        let foreign = format!("{USER_PATCH}- id: codex-plus-cu\n  name: something-else\n");
        assert!(install(Some(&foreign), &runtime(), path).is_err());
        assert!(install(Some("[{id: a}]\n"), &runtime(), path).is_err());
        assert!(install(Some("key: value\n"), &runtime(), path).is_err());
    }

    #[test]
    fn stale_block_is_rewritten_in_place_of_old_one() {
        let path = Path::new("/tmp/cordis.patch.yml");
        let installed = install(Some(USER_PATCH), &runtime(), path).unwrap().unwrap();
        // 模拟换了构建位置：块里的 command 指向旧二进制。
        let command = json(&Value::from(runtime().self_command()));
        let stale = installed.replace(&command, "\"/old/CodexPlusPlus\"");
        assert_ne!(stale, installed);
        let rewritten = install(Some(&stale), &runtime(), path).unwrap().unwrap();
        assert_eq!(rewritten.matches(BEGIN_TAG).count(), 1);
        assert!(!rewritten.contains("/old/"));
        assert_eq!(rewritten, installed);
    }

    #[test]
    fn unterminated_block_is_an_error() {
        let path = Path::new("/tmp/cordis.patch.yml");
        let broken = format!("{USER_PATCH}{BEGIN}\n- id: codex-plus-cu\n");
        assert!(install(Some(&broken), &runtime(), path).is_err());
    }
}
