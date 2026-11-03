//! 只替换有语义改动的 JSONC 值，原文中其它位置（包括注释与换行）不参与序列化。

use std::ops::Range;

use anyhow::{Context, bail, ensure};
use serde::Serialize;
use serde_json::Value;

use super::files::parse_jsonc;

struct Node {
    span: Range<usize>,
    kind: Kind,
}

enum Kind {
    Object(Vec<Member>),
    Array(Vec<Element>),
    Scalar,
}

struct Member {
    key: String,
    start: usize,
    key_end: usize,
    value: Node,
    comma: Option<usize>,
}

struct Element {
    leading: usize,
    value: Node,
    comma: Option<usize>,
}

struct Parser<'a> {
    text: &'a str,
    at: usize,
}

impl<'a> Parser<'a> {
    fn document(text: &'a str) -> anyhow::Result<Option<Node>> {
        let mut parser = Self {
            text,
            at: usize::from(text.starts_with('\u{feff}')) * '\u{feff}'.len_utf8(),
        };
        parser.trivia()?;
        if parser.at == text.len() {
            return Ok(None);
        }
        let node = parser.value()?;
        parser.trivia()?;
        ensure!(parser.at == text.len(), "JSONC 值之后还有其它内容");
        Ok(Some(node))
    }

    fn byte(&self) -> Option<u8> {
        self.text.as_bytes().get(self.at).copied()
    }

    fn trivia(&mut self) -> anyhow::Result<()> {
        loop {
            match self.byte() {
                Some(byte) if byte.is_ascii_whitespace() => self.at += 1,
                Some(b'/') if self.text.as_bytes().get(self.at + 1) == Some(&b'/') => {
                    self.at += 2;
                    while self.byte().is_some_and(|byte| byte != b'\n') {
                        self.at += 1;
                    }
                }
                Some(b'/') if self.text.as_bytes().get(self.at + 1) == Some(&b'*') => {
                    let end = self.text[self.at + 2..]
                        .find("*/")
                        .context("JSONC 块注释没有结束")?;
                    self.at += end + 4;
                }
                _ => return Ok(()),
            }
        }
    }

    fn take(&mut self, byte: u8) -> anyhow::Result<()> {
        ensure!(self.byte() == Some(byte), "JSONC 结构不完整");
        self.at += 1;
        Ok(())
    }

    fn string(&mut self) -> anyhow::Result<Range<usize>> {
        let start = self.at;
        self.take(b'"')?;
        loop {
            match self.byte() {
                Some(b'\\') => self.at += 2,
                Some(b'"') => {
                    self.at += 1;
                    return Ok(start..self.at);
                }
                Some(_) => self.at += 1,
                None => bail!("JSONC 字符串没有结束"),
            }
        }
    }

    fn value(&mut self) -> anyhow::Result<Node> {
        let start = self.at;
        let kind = match self.byte() {
            Some(b'{') => {
                self.at += 1;
                self.trivia()?;
                let mut members = Vec::new();
                while self.byte() != Some(b'}') {
                    let key_span = self.string()?;
                    let key = serde_json::from_str(&self.text[key_span.clone()])?;
                    self.trivia()?;
                    self.take(b':')?;
                    self.trivia()?;
                    let value = self.value()?;
                    self.trivia()?;
                    let comma = (self.byte() == Some(b',')).then_some(self.at);
                    if comma.is_some() {
                        self.at += 1;
                    }
                    members.push(Member {
                        key,
                        start: key_span.start,
                        key_end: key_span.end,
                        value,
                        comma,
                    });
                    self.trivia()?;
                    if comma.is_none() {
                        break;
                    }
                }
                self.take(b'}')?;
                Kind::Object(members)
            }
            Some(b'[') => {
                self.at += 1;
                let mut elements = Vec::new();
                loop {
                    let leading = self.at;
                    self.trivia()?;
                    if self.byte() == Some(b']') {
                        break;
                    }
                    let value = self.value()?;
                    self.trivia()?;
                    let comma = (self.byte() == Some(b',')).then_some(self.at);
                    if comma.is_some() {
                        self.at += 1;
                    }
                    elements.push(Element {
                        leading,
                        value,
                        comma,
                    });
                    if comma.is_none() {
                        self.trivia()?;
                        break;
                    }
                }
                self.take(b']')?;
                Kind::Array(elements)
            }
            Some(b'"') => {
                self.string()?;
                Kind::Scalar
            }
            Some(_) => {
                while self.byte().is_some_and(|byte| {
                    !byte.is_ascii_whitespace() && !matches!(byte, b',' | b'}' | b']' | b'/')
                }) {
                    self.at += 1;
                }
                ensure!(self.at > start, "JSONC 值不完整");
                Kind::Scalar
            }
            None => bail!("JSONC 值不完整"),
        };
        Ok(Node {
            span: start..self.at,
            kind,
        })
    }
}

struct Style {
    multiline: bool,
    newline: &'static str,
    indent: String,
    unit: String,
    closing_indent: String,
    gap: String,
    colon: &'static str,
}

fn line_indent(text: &str, at: usize) -> Option<&str> {
    let start = text[..at].rfind('\n').map_or(0, |at| at + 1);
    let prefix = &text[start..at];
    prefix
        .bytes()
        .all(|byte| matches!(byte, b' ' | b'\t'))
        .then_some(prefix)
}

impl Style {
    fn of(text: &str, node: &Node) -> Self {
        let starts: Vec<usize> = match &node.kind {
            Kind::Object(members) => members.iter().map(|member| member.start).collect(),
            Kind::Array(elements) => elements
                .iter()
                .map(|element| element.value.span.start)
                .collect(),
            Kind::Scalar => Vec::new(),
        };
        let closing_indent = line_indent(text, node.span.end - 1)
            .unwrap_or_default()
            .to_string();
        let known_indent = starts
            .iter()
            .find_map(|at| line_indent(text, *at))
            .filter(|indent| indent.len() > closing_indent.len());
        let unit = known_indent
            .and_then(|indent| indent.strip_prefix(&closing_indent))
            .unwrap_or(if text.contains("\n\t") { "\t" } else { "  " })
            .to_string();
        let indent = known_indent
            .map(str::to_string)
            .unwrap_or_else(|| format!("{closing_indent}{unit}"));
        let colon = match &node.kind {
            Kind::Object(members) => members
                .first()
                .map(|member| {
                    let between = &text[member.key_end..member.value.span.start];
                    if between.ends_with(':') { ":" } else { ": " }
                })
                .unwrap_or(": "),
            _ => ": ",
        };
        let gap = starts
            .get(1)
            .and_then(|second| {
                let comma = match &node.kind {
                    Kind::Object(members) => members[0].comma?,
                    Kind::Array(elements) => elements[0].comma?,
                    Kind::Scalar => return None,
                };
                let gap = &text[comma + 1..*second];
                gap.bytes()
                    .all(|byte| matches!(byte, b' ' | b'\t'))
                    .then_some(gap)
            })
            .unwrap_or(if colon == ":" { "" } else { " " })
            .to_string();
        Self {
            multiline: text[node.span.clone()].contains('\n'),
            newline: if text.contains("\r\n") { "\r\n" } else { "\n" },
            indent,
            unit,
            closing_indent,
            gap,
            colon,
        }
    }

    fn value(&self, value: &Value) -> anyhow::Result<String> {
        if !self.multiline {
            return Ok(serde_json::to_string(value)?);
        }
        let mut bytes = Vec::new();
        let formatter = serde_json::ser::PrettyFormatter::with_indent(self.unit.as_bytes());
        value.serialize(&mut serde_json::Serializer::with_formatter(
            &mut bytes, formatter,
        ))?;
        let serialized = String::from_utf8(bytes)?;
        Ok(serialized.replace('\n', &format!("{}{}", self.newline, self.indent)))
    }
}

fn apply(text: &mut String, mut edits: Vec<(Range<usize>, String)>) {
    edits.sort_by(|left, right| right.0.start.cmp(&left.0.start));
    for (span, replacement) in edits {
        text.replace_range(span, &replacement);
    }
}

fn root(text: &str) -> anyhow::Result<Node> {
    Parser::document(text)?.context("JSONC 文件没有值")
}

fn delete(
    text: &mut String,
    span: Range<usize>,
    comma: Option<usize>,
    previous_comma: Option<usize>,
) {
    let mut remove = span.start..comma.map_or(span.end, |comma| comma + 1);
    let line_start = text[..remove.start].rfind('\n').map_or(0, |at| at + 1);
    let line_end = text[remove.end..]
        .find('\n')
        .map_or(text.len(), |at| remove.end + at + 1);
    if text[line_start..remove.start].trim().is_empty()
        && text[remove.end..line_end].trim().is_empty()
    {
        remove = line_start..line_end;
    }
    let mut edits = vec![(remove, String::new())];
    if comma.is_none()
        && let Some(comma) = previous_comma
    {
        edits.push((comma..comma + 1, String::new()));
    }
    apply(text, edits);
}

fn append(text: &mut String, key: Option<&str>, value: &Value) -> anyhow::Result<()> {
    let node = root(text)?;
    let style = Style::of(text, &node);
    let (last, trailing) = match &node.kind {
        Kind::Object(members) => (
            members.last().map(|member| member.value.span.end),
            members.last().and_then(|member| member.comma).is_some(),
        ),
        Kind::Array(elements) => (
            elements.last().map(|element| element.value.span.end),
            elements.last().and_then(|element| element.comma).is_some(),
        ),
        Kind::Scalar => bail!("无法向 JSONC 标量追加条目"),
    };
    let close = node.span.end - 1;
    let mut rendered = match key {
        Some(key) => format!("{}{}", serde_json::to_string(key)?, style.colon),
        None => String::new(),
    };
    rendered.push_str(&style.value(value)?);
    if trailing {
        rendered.push(',');
    }
    let (at, inserted) = if style.multiline {
        let line_start = text[..close].rfind('\n').map_or(close, |at| at + 1);
        let on_own_line = text[line_start..close].trim().is_empty();
        let at = if on_own_line { line_start } else { close };
        let prefix = if text[..at].ends_with('\n') {
            ""
        } else {
            style.newline
        };
        (
            at,
            format!(
                "{prefix}{}{rendered}{}{}",
                style.indent,
                style.newline,
                if on_own_line {
                    ""
                } else {
                    &style.closing_indent
                },
            ),
        )
    } else {
        let at = text[..close].trim_end_matches([' ', '\t']).len();
        let at = at.max(node.span.start + 1);
        let gap = if last.is_some() {
            style.gap.as_str()
        } else {
            ""
        };
        (at, format!("{gap}{rendered}"))
    };
    let mut edits = vec![(at..at, inserted)];
    if let Some(last) = last.filter(|_| !trailing) {
        // 同一位置的追加要在值后面放逗号，避免排序相等时倒置。
        if last == at {
            edits[0].1.insert(0, ',');
        } else {
            edits.push((last..last, ",".to_string()));
        }
    }
    apply(text, edits);
    Ok(())
}

fn patch(text: &str, old: &Value, desired: &Value) -> anyhow::Result<String> {
    if old == desired {
        return Ok(text.to_string());
    }
    let node = root(text)?;
    let mut result = text.to_string();
    match (&node.kind, old, desired) {
        (Kind::Object(members), Value::Object(old), Value::Object(desired)) => {
            let mut edits = Vec::new();
            for (index, member) in members.iter().enumerate() {
                // 重复键由 JSON 的最后一个值生效，前面的原文保持原样。
                if members[index + 1..]
                    .iter()
                    .any(|next| next.key == member.key)
                {
                    continue;
                }
                if let (Some(before), Some(after)) =
                    (old.get(&member.key), desired.get(&member.key))
                    && before != after
                {
                    edits.push((
                        member.value.span.clone(),
                        patch(&text[member.value.span.clone()], before, after)?,
                    ));
                }
            }
            apply(&mut result, edits);
            loop {
                let node = root(&result)?;
                let Kind::Object(members) = node.kind else {
                    unreachable!()
                };
                let Some(index) = members
                    .iter()
                    .position(|member| !desired.contains_key(&member.key))
                else {
                    break;
                };
                let member = &members[index];
                delete(
                    &mut result,
                    member.start..member.value.span.end,
                    member.comma,
                    index.checked_sub(1).and_then(|index| members[index].comma),
                );
            }
            for (key, value) in desired {
                if !old.contains_key(key) {
                    append(&mut result, Some(key), value)?;
                }
            }
        }
        (Kind::Array(elements), Value::Array(old), Value::Array(desired)) => {
            let prefix = old.iter().zip(desired).take_while(|(a, b)| a == b).count();
            let suffix = old[prefix..]
                .iter()
                .rev()
                .zip(desired[prefix..].iter().rev())
                .take_while(|(a, b)| a == b)
                .count();
            let old_middle = old.len() - prefix - suffix;
            let new_middle = desired.len() - prefix - suffix;
            let common = old_middle.min(new_middle);
            let mut edits = Vec::new();
            for index in prefix..prefix + common {
                let span = elements[index].value.span.clone();
                edits.push((
                    span.clone(),
                    patch(&text[span], &old[index], &desired[index])?,
                ));
            }
            apply(&mut result, edits);
            for _ in common..old_middle {
                let node = root(&result)?;
                let Kind::Array(elements) = node.kind else {
                    unreachable!()
                };
                let index = prefix + common;
                let element = &elements[index];
                delete(
                    &mut result,
                    element.value.span.clone(),
                    element.comma,
                    index.checked_sub(1).and_then(|index| elements[index].comma),
                );
            }
            for index in prefix + common..prefix + new_middle {
                let node = root(&result)?;
                let Kind::Array(elements) = &node.kind else {
                    unreachable!()
                };
                if index == elements.len() {
                    append(&mut result, None, &desired[index])?;
                    continue;
                }
                let style = Style::of(&result, &node);
                let element = &elements[index];
                let prefix = if style.multiline {
                    format!("{}{}", style.newline, style.indent)
                } else if index > 0 {
                    style.gap.clone()
                } else {
                    String::new()
                };
                let mut inserted = format!("{prefix}{},", style.value(&desired[index])?);
                if style.multiline
                    && !result[element.leading..element.value.span.start].contains('\n')
                {
                    inserted.push_str(style.newline);
                    inserted.push_str(&style.indent);
                }
                result.insert_str(element.leading, &inserted);
            }
        }
        _ => {
            let style = Style::of(text, &node);
            result.replace_range(node.span, &style.value(desired)?);
        }
    }
    Ok(result)
}

/// `None` 表示新文件；已有文件只改有差异的值，输出后重新校验语义。
pub(super) fn render(original: Option<&str>, desired: &Value) -> anyhow::Result<String> {
    let Some(original) = original else {
        return Ok(format!("{}\n", serde_json::to_string_pretty(desired)?));
    };
    let old = parse_jsonc(original)?;
    if old == *desired {
        return Ok(original.to_string());
    }
    let rendered = if Parser::document(original)?.is_some() {
        patch(original, &old, desired)?
    } else {
        let newline = if original.contains("\r\n") {
            "\r\n"
        } else {
            "\n"
        };
        let separator = if original.trim_matches('\u{feff}').is_empty() || original.ends_with('\n')
        {
            ""
        } else {
            newline
        };
        format!(
            "{original}{separator}{}{newline}",
            serde_json::to_string_pretty(desired)?.replace('\n', newline),
        )
    };
    ensure!(
        parse_jsonc(&rendered)? == *desired,
        "JSONC 局部编辑结果与目标不一致，未写入"
    );
    Ok(rendered)
}
