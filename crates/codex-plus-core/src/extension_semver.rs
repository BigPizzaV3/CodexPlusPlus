//! 拓展清单 `engines.codexpp` 的 semver 范围校验（不引入依赖的小实现）。
//!
//! 支持：`*` / `x` 通配、部分版本（`2`、`2.1`）、`=` `>` `>=` `<` `<=`、`^`、`~`、连字符范围
//! （`1.2 - 2.3`）、空格表示「且」、`||` 表示「或」。预发布与构建元数据部分忽略。

use anyhow::Context;

type Version = (u64, u64, u64);

/// 解析完整版本号，例如 `2.1.4`、`v2.1.4-beta.1`（预发布部分忽略）。
pub fn parse_version(text: &str) -> anyhow::Result<Version> {
    let core = text.trim().trim_start_matches(['v', 'V']).split(['-', '+']).next().unwrap_or_default();
    let mut parts = core.split('.').map(|part| part.parse::<u64>());
    let major = parts.next().context("版本号为空")?.with_context(|| format!("版本号不合法：{text}"))?;
    let minor = parts.next().transpose().with_context(|| format!("版本号不合法：{text}"))?.unwrap_or(0);
    let patch = parts.next().transpose().with_context(|| format!("版本号不合法：{text}"))?.unwrap_or(0);
    Ok((major, minor, patch))
}

/// 部分版本：缺省或通配的段为 None。
fn parse_partial(text: &str) -> anyhow::Result<[Option<u64>; 3]> {
    let core = text.trim().trim_start_matches(['v', 'V']).split(['-', '+']).next().unwrap_or_default();
    let mut out = [None; 3];
    for (index, part) in core.split('.').enumerate() {
        if index > 2 {
            anyhow::bail!("版本号段数过多：{text}");
        }
        if matches!(part, "x" | "X" | "*" | "") {
            break;
        }
        out[index] = Some(part.parse::<u64>().with_context(|| format!("版本号不合法：{text}"))?);
    }
    Ok(out)
}

fn floor(partial: &[Option<u64>; 3]) -> Version {
    (partial[0].unwrap_or(0), partial[1].unwrap_or(0), partial[2].unwrap_or(0))
}

/// 部分版本之后的第一个版本：`2` → 3.0.0，`2.1` → 2.2.0，`2.1.4` → 2.1.5；全通配为 None（无上限）。
fn bump(partial: &[Option<u64>; 3]) -> Option<Version> {
    match partial {
        [Some(major), None, _] => Some((major + 1, 0, 0)),
        [Some(major), Some(minor), None] => Some((*major, minor + 1, 0)),
        [Some(major), Some(minor), Some(patch)] => Some((*major, *minor, patch + 1)),
        _ => None,
    }
}

/// 一个比较子展开成的区间：下限（含）与上限（不含）。
type Bounds = (Option<Version>, Option<Version>);

fn comparator(text: &str) -> anyhow::Result<Bounds> {
    let (op, rest) = ["<=", ">=", "<", ">", "=", "^", "~"]
        .iter()
        .find_map(|op| text.strip_prefix(op).map(|rest| (*op, rest)))
        .unwrap_or(("", text));
    let partial = parse_partial(rest)?;
    let wildcard = partial[0].is_none();
    Ok(match op {
        "" | "=" => {
            if wildcard {
                (None, None)
            } else {
                (Some(floor(&partial)), bump(&partial))
            }
        }
        ">=" => (Some(floor(&partial)), None),
        ">" => (Some(bump(&partial).unwrap_or((u64::MAX, 0, 0))), None),
        "<" => {
            if wildcard {
                (Some((u64::MAX, 0, 0)), None)
            } else {
                (None, Some(floor(&partial)))
            }
        }
        "<=" => (None, bump(&partial)),
        "~" => {
            let upper = match partial {
                [Some(major), Some(minor), _] => Some((major, minor + 1, 0)),
                [Some(major), None, _] => Some((major + 1, 0, 0)),
                _ => None,
            };
            (Some(floor(&partial)), upper)
        }
        "^" => {
            let upper = match partial {
                [Some(0), Some(0), Some(patch)] => Some((0, 0, patch + 1)),
                [Some(0), Some(minor), _] => Some((0, minor + 1, 0)),
                [Some(major), _, _] => Some((major + 1, 0, 0)),
                _ => None,
            };
            (Some(floor(&partial)), upper)
        }
        _ => unreachable!(),
    })
}

fn within(version: Version, (lower, upper): Bounds) -> bool {
    lower.is_none_or(|lower| version >= lower) && upper.is_none_or(|upper| version < upper)
}

/// `version` 是否落在 semver 范围 `range` 里。范围写错时报错。
pub fn satisfies(version: &str, range: &str) -> anyhow::Result<bool> {
    let version = parse_version(version)?;
    let range = range.trim();
    if range.is_empty() {
        anyhow::bail!("版本范围为空");
    }
    let mut any = false;
    for set in range.split("||") {
        let set = set.trim();
        let ok = if let Some((low, high)) = set.split_once(" - ") {
            // 连字符范围：下限按部分版本取底，上限含该部分版本整段。
            let low = parse_partial(low)?;
            let high = parse_partial(high)?;
            within(version, (Some(floor(&low)), bump(&high)))
        } else {
            // 运算符与版本之间允许空格（`>= 2.1`）：先把它们并到一起。
            let mut tokens = Vec::<String>::new();
            for token in set.split_whitespace() {
                match tokens.last_mut() {
                    Some(last) if ["<=", ">=", "<", ">", "=", "^", "~"].contains(&last.as_str()) => last.push_str(token),
                    _ => tokens.push(token.to_string()),
                }
            }
            if tokens.is_empty() {
                anyhow::bail!("版本范围不合法：{range}");
            }
            let mut ok = true;
            for token in &tokens {
                ok &= within(version, comparator(token)?);
            }
            ok
        };
        any |= ok;
    }
    Ok(any)
}

/// 当前 Codex++ 的版本号。
pub fn codexpp_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}
