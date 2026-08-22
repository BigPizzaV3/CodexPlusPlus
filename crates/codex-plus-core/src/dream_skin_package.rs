use anyhow::{Context, bail};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::io::{Cursor, Read};
use zip::ZipArchive;

const PACKAGE_LIMIT: usize = 32 * 1024 * 1024;
const UNPACKED_LIMIT: usize = 64 * 1024 * 1024;
const FILE_LIMIT: usize = 32;
const MANIFEST_LIMIT: usize = 65_536;
const THEME_LIMIT: usize = 1_048_576;
const CSS_LIMIT: usize = 262_144;
const IMAGE_LIMIT: usize = 10 * 1024 * 1024;
const LICENSE_LIMIT: usize = 65_536;
const SIGNATURE_LIMIT: usize = 4096;

const ALLOWED_FILES: &[&str] = &[
    "manifest.json",
    "manifest.sig",
    "theme.json",
    "theme.css",
    "background.webp",
    "background.jpg",
    "background.png",
    "LICENSE.txt",
];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct DreamSkinPackageManifest {
    pub package_version: u8,
    pub theme_id: String,
    pub version: String,
    pub skin_api_version: u8,
    pub min_client_version: String,
    pub platforms: Vec<String>,
    pub capabilities: Vec<String>,
    pub publisher: DreamSkinPackagePublisher,
    pub license: String,
    pub provenance: DreamSkinPackageProvenance,
    pub files: Vec<DreamSkinPackageFile>,
    pub created_at: String,
    #[serde(default)]
    pub key_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct DreamSkinPackagePublisher {
    pub id: String,
    pub display_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct DreamSkinPackageProvenance {
    pub ai_generated: bool,
    pub summary: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct DreamSkinPackageFile {
    pub path: String,
    pub media_type: String,
    pub bytes: usize,
    pub sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatedDreamSkinPackage {
    pub manifest: DreamSkinPackageManifest,
    pub theme: Value,
    pub image_name: String,
    pub image_bytes: Vec<u8>,
    pub css: String,
    pub manifest_bytes: Vec<u8>,
    pub license_bytes: Option<Vec<u8>>,
}

pub fn validate_and_read_package(
    bytes: &[u8],
    platform: &str,
) -> anyhow::Result<ValidatedDreamSkinPackage> {
    if bytes.is_empty() || bytes.len() > PACKAGE_LIMIT {
        bail!("Dream Skin 主题包超过 32 MiB 限制");
    }
    let mut archive =
        ZipArchive::new(Cursor::new(bytes)).context("Dream Skin 主题包不是有效 ZIP")?;
    if archive.len() == 0 || archive.len() > FILE_LIMIT {
        bail!("Dream Skin 主题包文件数量必须在 1 到 32 之间");
    }

    let mut entries = Vec::new();
    let mut unpacked = 0usize;
    for index in 0..archive.len() {
        let file = archive
            .by_index(index)
            .context("读取 Dream Skin ZIP 条目失败")?;
        let name = file.name().to_string();
        validate_archive_entry_name(&name)?;
        if file.encrypted() || file.is_symlink() {
            bail!("主题包包含加密内容或链接：{name}");
        }
        if file.is_dir() {
            entries.push((name, None));
            continue;
        }
        let base_name = name.rsplit('/').next().unwrap_or_default();
        if base_name == ".DS_Store" || name.starts_with("__MACOSX/") {
            entries.push((name, None));
            continue;
        }
        if !ALLOWED_FILES.contains(&base_name) {
            bail!("主题包包含不支持的文件：{name}");
        }
        let limit = file_limit(base_name);
        let mut content = Vec::new();
        file.take((limit + 1) as u64)
            .read_to_end(&mut content)
            .context("读取 Dream Skin ZIP 内容失败")?;
        if content.is_empty() || content.len() > limit {
            bail!("主题包文件 {name} 超过大小限制");
        }
        unpacked = unpacked.saturating_add(content.len());
        if unpacked > UNPACKED_LIMIT {
            bail!("Dream Skin 主题包解包后超过 64 MiB 限制");
        }
        entries.push((name, Some(content)));
    }

    let mut files = normalize_package_entries(entries)?;

    let manifest_bytes = files
        .remove("manifest.json")
        .context("主题包缺少 manifest.json")?;
    let theme_bytes = files
        .remove("theme.json")
        .context("主题包缺少 theme.json")?;
    let css_bytes = files.remove("theme.css").context("主题包缺少 theme.css")?;
    let image_name = ["background.webp", "background.jpg", "background.png"]
        .iter()
        .find(|name| files.contains_key(**name))
        .map(|name| (*name).to_string())
        .context("主题包必须包含一个背景图片")?;
    if ["background.webp", "background.jpg", "background.png"]
        .iter()
        .filter(|name| files.contains_key(**name))
        .count()
        != 1
    {
        bail!("主题包必须且只能包含一个背景图片");
    }
    let image_bytes = files.remove(&image_name).expect("image was checked above");
    validate_image_content(&image_name, &image_bytes)?;
    let license_bytes = files.remove("LICENSE.txt");
    let signature = files.remove("manifest.sig");
    if let Some(signature) = signature {
        if signature.len() > SIGNATURE_LIMIT {
            bail!("manifest.sig 超过大小限制");
        }
    }
    if !files.is_empty() {
        bail!("主题包包含未处理文件");
    }

    let manifest: DreamSkinPackageManifest = parse_json(&manifest_bytes, "manifest.json")?;
    validate_manifest(&manifest, platform)?;
    let theme: Value = parse_json(&theme_bytes, "theme.json")?;
    validate_theme(&theme, &manifest, &image_name)?;
    let css = String::from_utf8(css_bytes).context("theme.css 不是有效 UTF-8")?;
    validate_safe_css(&css)?;
    validate_manifest_files(
        &manifest,
        &manifest_bytes,
        &theme_bytes,
        css.as_bytes(),
        &image_name,
        &image_bytes,
        license_bytes.as_deref(),
    )?;

    Ok(ValidatedDreamSkinPackage {
        manifest,
        theme,
        image_name,
        image_bytes,
        css,
        manifest_bytes,
        license_bytes,
    })
}

pub fn validate_safe_css(css: &str) -> anyhow::Result<()> {
    parse_safe_css(css).map(|_| ())
}

/// Compile validated community CSS into the trusted cascade used by the renderer.
/// The package format deliberately forbids `!important`; the client adds it only
/// after validation so community rules cannot smuggle arbitrary declarations in.
pub fn compile_safe_css(css: &str) -> anyhow::Result<String> {
    let rules = parse_safe_css(css)?;
    let mut compiled = String::from("@layer dreamskin-community {\n");
    for rule in rules {
        compiled.push_str("  ");
        compiled.push_str(rule.selector);
        compiled.push_str(" {\n");
        for (property, value) in &rule.declarations {
            compiled.push_str("    ");
            compiled.push_str(property);
            compiled.push_str(": ");
            compiled.push_str(value);
            compiled.push_str(" !important;\n");
            if *property == "background-color" && matches!(rule.part, "sidebar" | "main" | "home") {
                compiled.push_str("    background-image: none !important;\n");
            }
        }
        compiled.push_str("  }\n");

        if rule.part == "root" {
            let bridged = rule
                .declarations
                .iter()
                .filter(|(property, _)| {
                    matches!(
                        *property,
                        "background-color"
                            | "color"
                            | "font-family"
                            | "font-size"
                            | "font-weight"
                            | "letter-spacing"
                            | "line-height"
                    )
                })
                .collect::<Vec<_>>();
            if !bridged.is_empty() {
                compiled.push_str("  ");
                compiled.push_str(rule.selector);
                compiled.push_str(" body {\n");
                for (property, value) in bridged {
                    compiled.push_str(&format!("    {property}: {value} !important;\n"));
                }
                compiled.push_str("  }\n");
            }
        }
        if rule.part == "composer-toolbar"
            && let Some((_, value)) = rule
                .declarations
                .iter()
                .find(|(property, _)| *property == "color")
        {
            compiled.push_str("  ");
            compiled.push_str(rule.selector);
            compiled.push_str(" :where(button:not([class~=\"bg-token-foreground\"]), button:not([class~=\"bg-token-foreground\"]) *) {\n");
            compiled.push_str(&format!("    color: {value} !important;\n  }}\n"));
        }
    }
    compiled.push_str("}\n");
    Ok(compiled)
}

#[derive(Debug)]
struct SafeCssRule<'a> {
    selector: &'a str,
    part: &'a str,
    declarations: Vec<(&'a str, &'a str)>,
}

fn parse_safe_css(css: &str) -> anyhow::Result<Vec<SafeCssRule<'_>>> {
    if css.is_empty() || css.len() > CSS_LIMIT {
        bail!("theme.css 为空或超过 256 KiB 限制");
    }
    if css.chars().any(forbidden_css_character)
        || css.contains("/*")
        || css.contains("*/")
        || css.contains('\\')
    {
        bail!("theme.css 包含不允许的控制字符、注释或转义");
    }
    let allowed_parts = [
        "root",
        "sidebar",
        "main",
        "header",
        "home",
        "home-hero",
        "project-list",
        "thread",
        "message",
        "composer",
        "composer-toolbar",
        "dialog",
    ];
    let mut parsed = Vec::new();
    let mut declarations = 0usize;
    let mut rest = css;
    loop {
        rest = rest.trim_start_matches(|character: char| {
            character.is_ascii_whitespace() || character == '\u{000c}'
        });
        if rest.is_empty() {
            break;
        }
        let open = rest.find('{').context("theme.css 规则缺少开始括号")?;
        let selector = rest[..open].trim();
        if selector.is_empty()
            || selector.contains([',', '@', ';', '}'])
            || rest[open + 1..].find('{').is_some_and(|nested| {
                rest[open + 1..]
                    .find('}')
                    .is_none_or(|close| nested < close)
            })
        {
            bail!("theme.css 选择器或规则语法无效");
        }
        let close = open
            + 1
            + rest[open + 1..]
                .find('}')
                .context("theme.css 规则缺少结束括号")?;
        let Some(after_prefix) = selector.strip_prefix("[data-ds-part=\"") else {
            bail!("theme.css 只能选择 Skin API 注册的 data-ds-part");
        };
        let (part, suffix) = after_prefix
            .split_once("\"]")
            .context("theme.css data-ds-part 语法无效")?;
        if !allowed_parts.contains(&part)
            || (!suffix.is_empty() && suffix != ":hover" && suffix != ":focus-visible")
        {
            bail!("theme.css 使用了未注册的 Skin API part 或状态");
        }

        let mut rule_declarations = Vec::new();
        let mut seen = HashSet::new();
        for declaration in rest[open + 1..close]
            .split(';')
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            let (property, value) = declaration
                .split_once(':')
                .context("theme.css 声明语法无效")?;
            let property = property.trim();
            let value = value.trim();
            if !property
                .bytes()
                .enumerate()
                .all(|(index, byte)| byte.is_ascii_lowercase() || (index > 0 && byte == b'-'))
                || !seen.insert(property)
            {
                bail!("theme.css CSS 属性无效或重复：{property}");
            }
            if value.is_empty()
                || value.chars().count() > 512
                || value.contains(['{', '}', '<', '>', '!', ';'])
                || !validate_property_value(property, value)
            {
                bail!("theme.css 属性值不受 Safe CSS 支持：{property}");
            }
            declarations += 1;
            if declarations > 512 {
                bail!("theme.css 声明数量超过限制");
            }
            rule_declarations.push((property, value));
        }
        if rule_declarations.is_empty() {
            bail!("theme.css 每条规则必须至少包含一个声明");
        }
        parsed.push(SafeCssRule {
            selector,
            part,
            declarations: rule_declarations,
        });
        if parsed.len() > 128 {
            bail!("theme.css 规则数量超过限制");
        }
        rest = &rest[close + 1..];
    }
    if parsed.is_empty() {
        bail!("theme.css 必须至少包含一条规则");
    }
    Ok(parsed)
}

fn forbidden_css_character(character: char) -> bool {
    matches!(
        character as u32,
        0x0000..=0x0008 | 0x000b | 0x000e..=0x001f | 0x007f..=0x009f
            | 0x2028 | 0x2029 | 0x200e | 0x200f | 0x202a..=0x202e | 0x2066..=0x2069 | 0xfeff
    )
}

fn validate_property_value(property: &str, value: &str) -> bool {
    const COLOR_PROPERTIES: &[&str] = &[
        "color",
        "background-color",
        "border-color",
        "border-top-color",
        "border-right-color",
        "border-bottom-color",
        "border-left-color",
    ];
    const WIDTH_PROPERTIES: &[&str] = &[
        "border-width",
        "border-top-width",
        "border-right-width",
        "border-bottom-width",
        "border-left-width",
    ];
    const STYLE_PROPERTIES: &[&str] = &[
        "border-style",
        "border-top-style",
        "border-right-style",
        "border-bottom-style",
        "border-left-style",
    ];
    const RADIUS_PROPERTIES: &[&str] = &[
        "border-radius",
        "border-top-left-radius",
        "border-top-right-radius",
        "border-bottom-right-radius",
        "border-bottom-left-radius",
    ];
    const SPACING_PROPERTIES: &[&str] = &["gap", "row-gap", "column-gap"];
    if COLOR_PROPERTIES.contains(&property) {
        return valid_css_color(value, property);
    }
    if WIDTH_PROPERTIES.contains(&property) {
        return repeated_values(value, 1, 4, |item| zero_or_px(item, 0.0, 4.0));
    }
    if STYLE_PROPERTIES.contains(&property) {
        return repeated_values(value, 1, 4, |item| {
            matches!(
                item.to_ascii_lowercase().as_str(),
                "none" | "solid" | "dashed" | "dotted"
            )
        });
    }
    if RADIUS_PROPERTIES.contains(&property) {
        return registered_var(value, &["--ds-theme-surface-radius"])
            || repeated_values(value, 1, 4, |item| zero_or_px(item, 0.0, 28.0));
    }
    if SPACING_PROPERTIES.contains(&property) {
        return zero_or_px(value, 0.0, 24.0);
    }
    match property {
        "box-shadow" => valid_box_shadow(value),
        "opacity" => {
            registered_var(value, &["--ds-theme-surface-opacity"]) || numeric(value, "", 0.65, 1.0)
        }
        "backdrop-filter" => valid_backdrop_filter(value),
        "font-family" => valid_font_family(value),
        "font-size" => numeric(value, "px", 12.0, 20.0),
        "font-weight" => matches!(
            value.to_ascii_lowercase().as_str(),
            "400" | "500" | "600" | "700" | "normal" | "bold"
        ),
        "line-height" => numeric(value, "", 1.1, 1.8),
        "letter-spacing" => value == "0" || numeric(value, "px", 0.0, 2.0),
        "transition-duration" => valid_transition_duration(value),
        "transition-property" => valid_transition_property(value),
        _ => false,
    }
}

fn split_top_level(value: &str, separator: char) -> Option<Vec<&str>> {
    let mut values = Vec::new();
    let mut start = 0usize;
    let mut depth = 0i32;
    for (index, character) in value.char_indices() {
        match character {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth < 0 {
                    return None;
                }
            }
            _ if character == separator && depth == 0 => {
                values.push(value[start..index].trim());
                start = index + character.len_utf8();
            }
            _ => {}
        }
    }
    if depth != 0 {
        return None;
    }
    values.push(value[start..].trim());
    Some(values)
}

fn split_whitespace(value: &str) -> Option<Vec<&str>> {
    let mut values = Vec::new();
    let mut start = None;
    let mut depth = 0i32;
    for (index, character) in value.char_indices() {
        match character {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth < 0 {
                    return None;
                }
            }
            _ => {}
        }
        if character.is_ascii_whitespace() && depth == 0 {
            if let Some(begin) = start.take() {
                values.push(&value[begin..index]);
            }
        } else if start.is_none() {
            start = Some(index);
        }
    }
    if depth != 0 {
        return None;
    }
    if let Some(begin) = start {
        values.push(&value[begin..]);
    }
    Some(values)
}

fn numeric(value: &str, unit: &str, minimum: f64, maximum: f64) -> bool {
    let number = if unit.is_empty() {
        value
    } else if value.len() >= unit.len()
        && value[value.len() - unit.len()..].eq_ignore_ascii_case(unit)
    {
        &value[..value.len() - unit.len()]
    } else {
        return false;
    };
    if !valid_number_syntax(number) {
        return false;
    }
    number
        .parse::<f64>()
        .is_ok_and(|number| number.is_finite() && number >= minimum && number <= maximum)
}

fn valid_number_syntax(value: &str) -> bool {
    let value = value.strip_prefix('-').unwrap_or(value);
    if value.is_empty() {
        return false;
    }
    let (integer, fraction) = value
        .split_once('.')
        .map_or((value, None), |(left, right)| (left, Some(right)));
    if fraction.is_some_and(|fraction| {
        fraction.is_empty() || !fraction.bytes().all(|byte| byte.is_ascii_digit())
    }) {
        return false;
    }
    if integer.is_empty() {
        return fraction.is_some();
    }
    integer.bytes().all(|byte| byte.is_ascii_digit())
        && (integer == "0" || !integer.starts_with('0'))
}

fn zero_or_px(value: &str, minimum: f64, maximum: f64) -> bool {
    value == "0" || numeric(value, "px", minimum, maximum)
}

fn registered_var(value: &str, allowed: &[&str]) -> bool {
    value
        .strip_prefix("var(")
        .and_then(|value| value.strip_suffix(')'))
        .map(str::trim)
        .is_some_and(|name| allowed.contains(&name))
}

fn valid_css_color(value: &str, property: &str) -> bool {
    const COLOR_VARIABLES: &[&str] = &[
        "--ds-theme-color-background",
        "--ds-theme-color-panel",
        "--ds-theme-color-panel-alt",
        "--ds-theme-color-accent",
        "--ds-theme-color-accent-alt",
        "--ds-theme-color-secondary",
        "--ds-theme-color-highlight",
        "--ds-theme-color-text",
        "--ds-theme-color-muted",
        "--ds-theme-color-line",
    ];
    if registered_var(value, COLOR_VARIABLES) {
        return true;
    }
    if let Some(hex) = value.strip_prefix('#') {
        return matches!(hex.len(), 3 | 4 | 6 | 8)
            && hex.bytes().all(|byte| byte.is_ascii_hexdigit());
    }
    if value.eq_ignore_ascii_case("currentcolor") {
        return true;
    }
    if value.eq_ignore_ascii_case("transparent") {
        return property != "color";
    }
    let (kind, inside) = if value.len() > 5
        && value
            .get(..4)
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case("rgb("))
        && value.ends_with(')')
    {
        ("rgb", &value[4..value.len() - 1])
    } else if value.len() > 6
        && value
            .get(..5)
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case("rgba("))
        && value.ends_with(')')
    {
        ("rgba", &value[5..value.len() - 1])
    } else {
        return false;
    };
    let Some(parts) = split_top_level(inside, ',') else {
        return false;
    };
    let expected = if kind == "rgb" { 3 } else { 4 };
    parts.len() == expected
        && parts[..3].iter().all(|part| color_channel(part))
        && (expected == 3 || alpha_channel(parts[3]))
}

fn color_channel(value: &str) -> bool {
    value.strip_suffix('%').map_or_else(
        || {
            value
                .parse::<u16>()
                .is_ok_and(|number| number <= 255 && (value == "0" || !value.starts_with('0')))
        },
        |number| numeric(number, "", 0.0, 100.0),
    )
}

fn alpha_channel(value: &str) -> bool {
    value.strip_suffix('%').map_or_else(
        || numeric(value, "", 0.0, 1.0),
        |number| numeric(number, "", 0.0, 100.0),
    )
}

fn repeated_values(
    value: &str,
    minimum: usize,
    maximum: usize,
    validator: impl Fn(&str) -> bool,
) -> bool {
    split_whitespace(value).is_some_and(|items| {
        (minimum..=maximum).contains(&items.len()) && items.into_iter().all(validator)
    })
}

fn valid_box_shadow(value: &str) -> bool {
    if value.eq_ignore_ascii_case("none") {
        return true;
    }
    split_top_level(value, ',').is_some_and(|shadows| {
        (1..=2).contains(&shadows.len())
            && shadows.into_iter().all(|shadow| {
                let Some(mut values) = split_whitespace(shadow) else {
                    return false;
                };
                if values
                    .first()
                    .is_some_and(|value| value.eq_ignore_ascii_case("inset"))
                {
                    values.remove(0);
                }
                if !(3..=5).contains(&values.len()) {
                    return false;
                }
                let color = values.pop().unwrap_or_default();
                valid_css_color(color, "box-shadow")
                    && (2..=4).contains(&values.len())
                    && zero_or_px(values[0], -32.0, 32.0)
                    && zero_or_px(values[1], -32.0, 32.0)
                    && values
                        .get(2)
                        .is_none_or(|value| zero_or_px(value, 0.0, 48.0))
                    && values
                        .get(3)
                        .is_none_or(|value| zero_or_px(value, -8.0, 16.0))
            })
    })
}

fn valid_font_family(value: &str) -> bool {
    split_top_level(value, ',').is_some_and(|families| {
        !families.is_empty()
            && families.len() <= 4
            && families.into_iter().all(|family| {
                matches!(
                    family.to_ascii_lowercase().as_str(),
                    "system-ui"
                        | "-apple-system"
                        | "blinkmacsystemfont"
                        | "ui-sans-serif"
                        | "ui-rounded"
                        | "ui-serif"
                        | "ui-monospace"
                        | "sans-serif"
                        | "serif"
                        | "monospace"
                )
            })
    })
}

fn valid_transition_duration(value: &str) -> bool {
    split_top_level(value, ',').is_some_and(|durations| {
        !durations.is_empty()
            && durations.len() <= 4
            && durations.into_iter().all(|duration| {
                duration == "0"
                    || numeric(duration, "ms", 0.0, 400.0)
                    || numeric(duration, "s", 0.0, 0.4)
            })
    })
}

fn transition_targets() -> &'static [&'static str] {
    &[
        "color",
        "background-color",
        "border-color",
        "border-top-color",
        "border-right-color",
        "border-bottom-color",
        "border-left-color",
        "border-width",
        "border-top-width",
        "border-right-width",
        "border-bottom-width",
        "border-left-width",
        "border-radius",
        "border-top-left-radius",
        "border-top-right-radius",
        "border-bottom-right-radius",
        "border-bottom-left-radius",
        "gap",
        "row-gap",
        "column-gap",
        "box-shadow",
        "opacity",
        "backdrop-filter",
        "font-size",
        "font-weight",
        "line-height",
        "letter-spacing",
    ]
}

fn valid_transition_property(value: &str) -> bool {
    split_top_level(value, ',').is_some_and(|properties| {
        !properties.is_empty()
            && properties.len() <= 4
            && properties
                .into_iter()
                .all(|property| transition_targets().contains(&property))
    })
}

fn valid_backdrop_filter(value: &str) -> bool {
    if value.eq_ignore_ascii_case("none") {
        return true;
    }
    let Some(filters) = split_whitespace(value) else {
        return false;
    };
    if filters.is_empty() || filters.len() > 4 {
        return false;
    }
    let mut seen = HashSet::new();
    for (index, filter) in filters.into_iter().enumerate() {
        let Some((name, argument)) = filter.split_once('(') else {
            return false;
        };
        let Some(argument) = argument.strip_suffix(')') else {
            return false;
        };
        let name = name.to_ascii_lowercase();
        let argument = argument.trim();
        if !seen.insert(name.clone()) {
            return false;
        }
        let valid = match name.as_str() {
            "blur" => {
                index == 0
                    && (registered_var(argument, &["--ds-theme-surface-blur"])
                        || zero_or_px(argument, 0.0, 30.0))
            }
            "saturate" => numeric(argument, "", 0.5, 2.0),
            "brightness" | "contrast" => numeric(argument, "", 0.8, 1.5),
            _ => false,
        };
        if !valid {
            return false;
        }
    }
    seen.contains("blur")
}

fn parse_json<T: serde::de::DeserializeOwned>(bytes: &[u8], label: &str) -> anyhow::Result<T> {
    serde_json::from_slice(bytes).with_context(|| format!("{label} 不是有效 JSON"))
}

fn validate_archive_entry_name(name: &str) -> anyhow::Result<()> {
    if name.is_empty() || name.starts_with('/') || name.contains('\\') {
        bail!("主题包路径不安全：{name}");
    }
    if name != name.trim()
        || name.contains(['\0', ':'])
        || name.split('/').any(|part| part == "." || part == "..")
        || (!name.starts_with("__MACOSX/")
            && name.split('/').filter(|part| !part.is_empty()).count() > 2)
    {
        bail!("主题包文件名不安全：{name}");
    }
    Ok(())
}

fn normalize_package_entries(
    entries: Vec<(String, Option<Vec<u8>>)>,
) -> anyhow::Result<std::collections::BTreeMap<String, Vec<u8>>> {
    let content_entries = entries
        .into_iter()
        .filter_map(|(name, content)| content.map(|content| (name, content)))
        .collect::<Vec<_>>();
    if content_entries.is_empty() {
        bail!("Dream Skin 主题包为空");
    }

    let root_files = content_entries.iter().any(|(name, _)| !name.contains('/'));
    let prefix = if root_files {
        if content_entries.iter().any(|(name, _)| name.contains('/')) {
            bail!("主题包文件必须全部位于根目录或唯一一层主题目录");
        }
        None
    } else {
        let prefix = content_entries[0]
            .0
            .split_once('/')
            .map(|(prefix, _)| prefix)
            .context("主题包顶级目录无效")?;
        if prefix.is_empty()
            || content_entries.iter().any(|(name, _)| {
                name.split_once('/').is_none_or(|(candidate, file)| {
                    candidate != prefix || file.is_empty() || file.contains('/')
                })
            })
        {
            bail!("主题包只能包含唯一一层主题目录");
        }
        Some(format!("{prefix}/"))
    };

    let mut files = std::collections::BTreeMap::new();
    for (name, content) in content_entries {
        let normalized = prefix
            .as_deref()
            .and_then(|prefix| name.strip_prefix(prefix))
            .unwrap_or(&name)
            .to_string();
        if files.insert(normalized.clone(), content).is_some() {
            bail!("主题包包含重复文件：{normalized}");
        }
    }
    Ok(files)
}

fn validate_image_content(name: &str, bytes: &[u8]) -> anyhow::Result<()> {
    let valid = match name {
        "background.png" => bytes.starts_with(&[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]),
        "background.jpg" => bytes.starts_with(&[0xff, 0xd8, 0xff]),
        "background.webp" => {
            bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP"
        }
        _ => false,
    };
    if !valid {
        bail!("主题包背景图片内容与扩展名不一致：{name}");
    }
    Ok(())
}

fn file_limit(name: &str) -> usize {
    match name {
        "manifest.json" => MANIFEST_LIMIT,
        "theme.json" => THEME_LIMIT,
        "theme.css" => CSS_LIMIT,
        "LICENSE.txt" => LICENSE_LIMIT,
        "manifest.sig" => SIGNATURE_LIMIT,
        _ => IMAGE_LIMIT,
    }
}

fn validate_manifest(manifest: &DreamSkinPackageManifest, platform: &str) -> anyhow::Result<()> {
    if manifest.package_version != 1 || manifest.skin_api_version != 1 {
        bail!("不支持的 Dream Skin 主题协议版本");
    }
    if manifest.theme_id.chars().count() < 3
        || manifest.theme_id.chars().count() > 64
        || !valid_theme_id(&manifest.theme_id)
    {
        bail!("manifest.themeId 无效");
    }
    if !valid_semver(&manifest.version) || !valid_semver(&manifest.min_client_version) {
        bail!("manifest 版本号无效");
    }
    if compare_semver(&manifest.min_client_version, "1.5.12").is_gt() {
        bail!(
            "主题包需要更新版本的 Dream Skin 协议：{}",
            manifest.min_client_version
        );
    }
    if manifest.platforms.is_empty()
        || manifest.platforms.len() > 2
        || has_duplicates(&manifest.platforms)
        || manifest
            .platforms
            .iter()
            .any(|item| !matches!(item.as_str(), "macos" | "windows"))
    {
        bail!("manifest.platforms 无效");
    }
    if !manifest.platforms.iter().any(|item| item == platform) {
        bail!("主题包不支持当前平台：{platform}");
    }
    if manifest.capabilities.is_empty()
        || manifest.capabilities.len() > 3
        || has_duplicates(&manifest.capabilities)
        || manifest
            .capabilities
            .iter()
            .any(|item| !matches!(item.as_str(), "background" | "tokens" | "safe-css"))
        || !manifest.capabilities.iter().any(|item| item == "safe-css")
    {
        bail!("manifest.capabilities 无效或缺少 safe-css 能力");
    }
    if !safe_manifest_text(&manifest.publisher.display_name, 1, 80, false)
        || manifest.publisher.id.is_empty()
        || manifest.publisher.id.len() > 64
        || !manifest
            .publisher
            .id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
        || manifest.license.is_empty()
        || manifest.license.len() > 64
        || !manifest.license.bytes().enumerate().all(|(index, byte)| {
            byte.is_ascii_alphanumeric()
                || (index > 0 && matches!(byte, b' ' | b'.' | b'+' | b'(' | b')' | b'-'))
        })
        || !safe_manifest_text(&manifest.provenance.summary, 1, 500, true)
    {
        bail!("manifest 发布者、许可证或来源说明为空");
    }
    if manifest
        .key_id
        .as_deref()
        .is_some_and(|key| !valid_key_id(key))
        || !valid_rfc3339(&manifest.created_at)
    {
        bail!("manifest keyId 或 createdAt 无效");
    }
    if manifest.files.len() < 2 || manifest.files.len() > 8 {
        bail!("manifest.files 数量无效");
    }
    Ok(())
}

fn validate_theme(
    theme: &Value,
    manifest: &DreamSkinPackageManifest,
    image_name: &str,
) -> anyhow::Result<()> {
    let object = theme.as_object().context("theme.json 必须是对象")?;
    const REQUIRED: &[&str] = &["schemaVersion", "id", "name", "image"];
    const OPTIONAL: &[&str] = &[
        "brandSubtitle",
        "tagline",
        "projectPrefix",
        "projectLabel",
        "statusText",
        "quote",
        "promoTitle",
        "promoSub",
        "promoUrl",
        "appearance",
        "art",
        "colors",
    ];
    if REQUIRED.iter().any(|key| !object.contains_key(*key))
        || object
            .keys()
            .any(|key| !REQUIRED.contains(&key.as_str()) && !OPTIONAL.contains(&key.as_str()))
    {
        bail!("theme.json 包含缺失或不支持的字段");
    }
    if object.get("schemaVersion").and_then(Value::as_u64) != Some(1)
        || object.get("id").and_then(Value::as_str) != Some(manifest.theme_id.as_str())
        || object.get("name").and_then(Value::as_str).is_none()
        || object.get("image").and_then(Value::as_str) != Some(image_name)
    {
        bail!("theme.json 与 manifest 身份或背景图不一致");
    }
    if object
        .get("name")
        .and_then(Value::as_str)
        .is_some_and(|name| !safe_manifest_text(name, 1, 80, false))
    {
        bail!("theme.json.name 无效");
    }
    for key in [
        "brandSubtitle",
        "tagline",
        "projectPrefix",
        "projectLabel",
        "statusText",
        "quote",
        "promoTitle",
        "promoSub",
    ] {
        if object.get(key).is_some_and(|value| {
            value
                .as_str()
                .is_none_or(|text| !safe_manifest_text(text, 0, 120, false))
        }) {
            bail!("theme.json.{key} 无效");
        }
    }
    if object.get("promoUrl").is_some_and(|value| {
        value
            .as_str()
            .is_none_or(|text| !safe_manifest_text(text, 0, 512, false))
    }) || object.get("appearance").is_some_and(|value| {
        value
            .as_str()
            .is_none_or(|appearance| !matches!(appearance, "auto" | "light" | "dark"))
    }) {
        bail!("theme.json 的链接或外观字段无效");
    }
    if let Some(art) = object.get("art") {
        validate_theme_art(art)?;
    }
    if let Some(colors) = object.get("colors") {
        let colors = colors.as_object().context("theme.json.colors 必须是对象")?;
        let keys = [
            "background",
            "panel",
            "panelAlt",
            "accent",
            "accentAlt",
            "secondary",
            "highlight",
            "text",
            "muted",
            "line",
        ];
        if colors.len() != keys.len() {
            bail!("theme.json.colors 字段无效");
        }
        for key in keys {
            if colors
                .get(key)
                .and_then(Value::as_str)
                .is_none_or(|value| !valid_theme_color(value))
            {
                bail!("theme.json.colors 缺少 {key}");
            }
        }
    }
    Ok(())
}

fn validate_manifest_files(
    manifest: &DreamSkinPackageManifest,
    manifest_bytes: &[u8],
    theme_bytes: &[u8],
    css_bytes: &[u8],
    image_name: &str,
    image_bytes: &[u8],
    license_bytes: Option<&[u8]>,
) -> anyhow::Result<()> {
    let mut seen = HashSet::new();
    for file in &manifest.files {
        if !seen.insert(file.path.as_str())
            || !ALLOWED_FILES.contains(&file.path.as_str())
            || matches!(file.path.as_str(), "manifest.json" | "manifest.sig")
            || file.bytes == 0
            || file.bytes > file_limit(&file.path)
            || file.sha256.len() != 64
            || !file
                .sha256
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            bail!("manifest.files 包含无效条目");
        }
        let bytes = match file.path.as_str() {
            "manifest.json" => manifest_bytes,
            "theme.json" => theme_bytes,
            "theme.css" => css_bytes,
            path if path == image_name => image_bytes,
            "LICENSE.txt" => license_bytes.context("manifest 声明了 LICENSE.txt 但包内缺失")?,
            "manifest.sig" => continue,
            _ => continue,
        };
        let expected_media_type = match file.path.as_str() {
            "theme.json" => "application/json",
            "theme.css" => "text/css",
            "background.webp" => "image/webp",
            "background.jpg" => "image/jpeg",
            "background.png" => "image/png",
            "LICENSE.txt" => "text/plain",
            _ => "",
        };
        if file.media_type != expected_media_type {
            bail!("manifest.files 的 mediaType 不匹配：{}", file.path);
        }
        let actual = format!("{:x}", Sha256::digest(bytes));
        if file.bytes != bytes.len() || !actual.eq_ignore_ascii_case(&file.sha256) {
            bail!("manifest.files 的 SHA-256 或大小不匹配：{}", file.path);
        }
    }
    if !seen.contains("theme.json") || !seen.contains("theme.css") || !seen.contains(image_name) {
        bail!("manifest.files 缺少必需文件");
    }
    if seen.contains("LICENSE.txt") != license_bytes.is_some() {
        bail!("manifest.files 的 LICENSE.txt 与包内容不一致");
    }
    Ok(())
}

fn valid_theme_id(value: &str) -> bool {
    let mut chars = value.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphanumeric())
        && value
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '-' | '.'))
}

fn valid_semver(value: &str) -> bool {
    let parts = value.split('.').collect::<Vec<_>>();
    parts.len() == 3
        && value.len() <= 32
        && parts.iter().all(|part| {
            !part.is_empty()
                && part.bytes().all(|byte| byte.is_ascii_digit())
                && (*part == "0" || !part.starts_with('0'))
        })
}

fn compare_semver(left: &str, right: &str) -> std::cmp::Ordering {
    for (left, right) in left.split('.').zip(right.split('.')) {
        let ordering = left.len().cmp(&right.len()).then_with(|| left.cmp(right));
        if !ordering.is_eq() {
            return ordering;
        }
    }
    std::cmp::Ordering::Equal
}

fn has_duplicates(values: &[String]) -> bool {
    let mut seen = HashSet::new();
    values.iter().any(|value| !seen.insert(value))
}

fn safe_manifest_text(value: &str, minimum: usize, maximum: usize, allow_lines: bool) -> bool {
    let count = value.chars().count();
    count >= minimum
        && count <= maximum
        && value.chars().all(|character| {
            let code = character as u32;
            if allow_lines && matches!(character, '\n' | '\t') {
                return true;
            }
            !matches!(code, 0x0000..=0x001f | 0x007f)
        })
}

fn valid_key_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value.split('-').all(|part| {
            !part.is_empty()
                && part
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
        })
}

fn valid_rfc3339(value: &str) -> bool {
    if value.len() < 20 || !value.is_ascii() {
        return false;
    }
    let bytes = value.as_bytes();
    if bytes.get(4) != Some(&b'-')
        || bytes.get(7) != Some(&b'-')
        || bytes.get(10) != Some(&b'T')
        || bytes.get(13) != Some(&b':')
        || bytes.get(16) != Some(&b':')
    {
        return false;
    }
    let number = |range: std::ops::Range<usize>| value.get(range)?.parse::<u32>().ok();
    let Some(year) = number(0..4) else {
        return false;
    };
    let Some(month) = number(5..7) else {
        return false;
    };
    let Some(day) = number(8..10) else {
        return false;
    };
    let Some(hour) = number(11..13) else {
        return false;
    };
    let Some(minute) = number(14..16) else {
        return false;
    };
    let Some(second) = number(17..19) else {
        return false;
    };
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let month_days = [
        31,
        if leap { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    if !(1..=12).contains(&month)
        || day == 0
        || day > month_days[(month - 1) as usize]
        || hour > 23
        || minute > 59
        || second > 59
    {
        return false;
    }
    let mut rest = &value[19..];
    if let Some(fraction) = rest.strip_prefix('.') {
        let digits = fraction
            .bytes()
            .take_while(|byte| byte.is_ascii_digit())
            .count();
        if digits == 0 || digits > 9 {
            return false;
        }
        rest = &fraction[digits..];
    }
    if rest == "Z" {
        return true;
    }
    if rest.len() != 6 || !matches!(rest.as_bytes()[0], b'+' | b'-') || rest.as_bytes()[3] != b':' {
        return false;
    }
    rest[1..3].parse::<u32>().is_ok_and(|hour| hour <= 23)
        && rest[4..6].parse::<u32>().is_ok_and(|minute| minute <= 59)
}

fn validate_theme_art(value: &Value) -> anyhow::Result<()> {
    let object = value.as_object().context("theme.json.art 必须是对象")?;
    if object
        .keys()
        .any(|key| !matches!(key.as_str(), "focusX" | "focusY" | "safeArea" | "taskMode"))
    {
        bail!("theme.json.art 包含不支持的字段");
    }
    for key in ["focusX", "focusY"] {
        if object.get(key).is_some_and(|value| {
            value
                .as_f64()
                .is_none_or(|number| !number.is_finite() || !(0.0..=1.0).contains(&number))
        }) {
            bail!("theme.json.art.{key} 无效");
        }
    }
    if object.get("safeArea").is_some_and(|value| {
        value
            .as_str()
            .is_none_or(|value| !matches!(value, "left" | "right" | "none"))
    }) || object.get("taskMode").is_some_and(|value| {
        value
            .as_str()
            .is_none_or(|value| !matches!(value, "ambient" | "full" | "off"))
    }) {
        bail!("theme.json.art 枚举值无效");
    }
    Ok(())
}

fn valid_theme_color(value: &str) -> bool {
    if let Some(hex) = value.strip_prefix('#') {
        return matches!(hex.len(), 3 | 4 | 6 | 8)
            && hex.bytes().all(|byte| byte.is_ascii_hexdigit());
    }
    if value.starts_with("rgb(") && value.ends_with(')') {
        let values = &value[4..value.len() - 1];
        return split_top_level(values, ',').is_some_and(|values| {
            values.len() == 3
                && values.into_iter().all(|value| {
                    let value = value.trim();
                    !value.is_empty()
                        && value.len() <= 3
                        && value.bytes().all(|byte| byte.is_ascii_digit())
                })
        });
    }
    if value.starts_with("rgba(") && value.ends_with(')') {
        let values = &value[5..value.len() - 1];
        return split_top_level(values, ',').is_some_and(|values| {
            values.len() == 4
                && values[..3].iter().all(|value| {
                    let value = value.trim();
                    !value.is_empty()
                        && value.len() <= 3
                        && value.bytes().all(|byte| byte.is_ascii_digit())
                })
                && {
                    let alpha = values[3].trim();
                    matches!(alpha, "0" | "1" | "1.0")
                        || alpha.strip_prefix("0.").is_some_and(|digits| {
                            (1..=6).contains(&digits.len())
                                && digits.bytes().all(|byte| byte.is_ascii_digit())
                        })
                        || alpha.strip_prefix('.').is_some_and(|digits| {
                            (1..=6).contains(&digits.len())
                                && digits.bytes().all(|byte| byte.is_ascii_digit())
                        })
                }
        });
    }
    false
}
