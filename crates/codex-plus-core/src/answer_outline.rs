use serde::Serialize;

const DEFAULT_MAX_ITEMS: usize = 24;
const MAX_TITLE_CHARS: usize = 56;
const MIN_TITLE_CHARS: usize = 2;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AnswerOutlineItem {
    pub level: u8,
    pub title: String,
    pub number_prefix: String,
    pub source_line: usize,
}

/// Extracts a conservative outline from assistant Markdown or plain text.
///
/// The parser deliberately accepts only line-oriented headings. It does not
/// inspect the DOM, call a model, or infer structure from arbitrary prose.
pub fn extract(text: &str, max_items: usize) -> Vec<AnswerOutlineItem> {
    let limit = max_items.clamp(1, DEFAULT_MAX_ITEMS);
    let mut items = Vec::new();
    let mut in_code_block = false;
    let mut seen = std::collections::HashSet::new();

    for (line_index, raw_line) in text.lines().enumerate() {
        let line = raw_line.trim();
        if line.starts_with("```") || line.starts_with("~~~") {
            in_code_block = !in_code_block;
            continue;
        }
        if in_code_block || line.is_empty() {
            continue;
        }

        let Some(candidate) = heading_candidate(line) else {
            continue;
        };
        let key = normalize_key(&candidate.title);
        if !seen.insert(key) {
            continue;
        }

        items.push(AnswerOutlineItem {
            level: candidate.level,
            title: truncate_title(&candidate.title),
            number_prefix: candidate.number_prefix,
            source_line: line_index + 1,
        });
        if items.len() >= limit {
            break;
        }
    }

    normalize_levels(&mut items);
    items
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct HeadingCandidate {
    level: u8,
    title: String,
    number_prefix: String,
}

fn heading_candidate(line: &str) -> Option<HeadingCandidate> {
    if line.starts_with("- ") || line.starts_with("* ") || line.starts_with("> ") {
        return None;
    }

    let (level, content) = markdown_heading(line);
    let (number_prefix, title) = split_number_prefix(content);
    let title = truncate_title(&clean_title(title));
    if !is_valid_title(&title) {
        return None;
    }

    let inferred_level = if level > 0 {
        level
    } else if !number_prefix.is_empty() {
        number_prefix_level(&number_prefix)
    } else if is_chapter_title(&title) {
        2
    } else {
        return None;
    };

    Some(HeadingCandidate {
        level: inferred_level.clamp(1, 6),
        title,
        number_prefix,
    })
}

fn markdown_heading(line: &str) -> (u8, &str) {
    let hash_count = line
        .chars()
        .take_while(|character| *character == '#')
        .count();
    if (1..=6).contains(&hash_count) {
        let marker = "#".repeat(hash_count);
        let content = line.strip_prefix(&marker).unwrap_or(line).trim_start();
        if !content.is_empty() {
            return (hash_count as u8, content.trim_end_matches('#').trim());
        }
    }
    (0, line)
}

fn split_number_prefix(value: &str) -> (String, &str) {
    let value = value.trim();
    if let Some((end, _)) = value
        .char_indices()
        .find(|(_, character)| character.is_whitespace())
    {
        let prefix = &value[..end];
        if is_number_prefix(prefix) {
            return (prefix.to_string(), value[end..].trim_start());
        }
    }

    for separator in ['、', '．', '.'] {
        if let Some(end) = value.find(separator) {
            let prefix_end = end + separator.len_utf8();
            let prefix = &value[..prefix_end];
            if prefix.chars().all(|character| {
                character.is_ascii_digit()
                    || character == separator
                    || character == '一'
                    || character == '二'
                    || character == '三'
                    || character == '四'
                    || character == '五'
                    || character == '六'
                    || character == '七'
                    || character == '八'
                    || character == '九'
                    || character == '十'
            }) {
                return (prefix.to_string(), value[prefix_end..].trim_start());
            }
        }
    }

    (String::new(), value)
}

fn is_number_prefix(value: &str) -> bool {
    let trimmed = value.trim_end_matches(['.', '、', '．', ')', '）']);
    !trimmed.is_empty()
        && trimmed.chars().all(|character| {
            character.is_ascii_digit()
                || ".、．)）".contains(character)
                || "一二三四五六七八九十百零".contains(character)
        })
}

fn number_prefix_level(prefix: &str) -> u8 {
    let depth = prefix
        .chars()
        .filter(|character| *character == '.' || *character == '．')
        .count();
    (depth + 1).clamp(1, 6) as u8
}

fn clean_title(value: &str) -> String {
    value
        .trim()
        .trim_end_matches([':', '：'])
        .trim()
        .to_string()
}

fn is_valid_title(title: &str) -> bool {
    let char_count = title.chars().count();
    if !(MIN_TITLE_CHARS..=MAX_TITLE_CHARS).contains(&char_count) {
        return false;
    }
    if title.starts_with("http://") || title.starts_with("https://") {
        return false;
    }
    if title.chars().all(|character| {
        character.is_ascii_digit() || character.is_ascii_punctuation() || character.is_whitespace()
    }) {
        return false;
    }
    !matches!(
        title.to_ascii_lowercase().as_str(),
        "ok" | "pass" | "fail" | "true" | "false"
    )
}

fn is_chapter_title(title: &str) -> bool {
    const CHAPTER_TITLES: &[&str] = &[
        "摘要",
        "概述",
        "背景",
        "目标",
        "问题",
        "原因",
        "分析",
        "方案",
        "步骤",
        "实现",
        "验证",
        "测试",
        "结果",
        "结论",
        "总结",
        "建议",
        "下一步",
        "说明",
        "附录",
        "abstract",
        "overview",
        "background",
        "goals",
        "analysis",
        "solution",
        "steps",
        "implementation",
        "verification",
        "tests",
        "results",
        "conclusion",
        "summary",
        "recommendations",
        "appendix",
        "next steps",
    ];
    CHAPTER_TITLES
        .iter()
        .any(|candidate| title.eq_ignore_ascii_case(candidate))
}

fn normalize_key(value: &str) -> String {
    value
        .chars()
        .filter(|character| !character.is_whitespace() && !matches!(character, '：' | ':'))
        .flat_map(char::to_lowercase)
        .collect()
}

fn truncate_title(value: &str) -> String {
    value.chars().take(MAX_TITLE_CHARS).collect()
}

fn normalize_levels(items: &mut [AnswerOutlineItem]) {
    let Some(minimum) = items.iter().map(|item| item.level).min() else {
        return;
    };
    for item in items {
        item.level = item.level.saturating_sub(minimum).saturating_add(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_markdown_and_numbered_headings() {
        let items = extract(
            "# 总结\n\n1. 现状\n\n## 1.1 实现\n\n二、验证\n\n普通段落",
            24,
        );

        assert_eq!(
            items
                .iter()
                .map(|item| item.title.as_str())
                .collect::<Vec<_>>(),
            ["总结", "现状", "实现", "验证"]
        );
        assert_eq!(items[0].level, 1);
        assert_eq!(items[2].level, 2);
        assert_eq!(items[1].number_prefix, "1.");
    }

    #[test]
    fn ignores_code_blocks_noise_and_duplicate_titles() {
        let items = extract(
            "```md\n# 不应出现\n```\n\n## 结果\n\n### 结果\n\nhttps://example.com\n\nPASS",
            24,
        );

        assert_eq!(items.len(), 1);
        assert_eq!(items[0].title, "结果");
        assert_eq!(items[0].source_line, 5);
    }

    #[test]
    fn clamps_max_items_and_title_length() {
        let long_title = format!("# {}", "标题".repeat(40));
        let items = extract(&format!("{long_title}\n# 第二个标题\n# 第三个标题"), 100);

        assert_eq!(items.len(), 3);
        assert_eq!(items[0].title.chars().count(), MAX_TITLE_CHARS);
        assert!(extract("# 一个标题\n# 第二个标题", 0).len() <= 1);
    }
}
