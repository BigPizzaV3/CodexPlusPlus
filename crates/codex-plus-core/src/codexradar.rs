use serde_json::{Value, json};

const CODEX_RADAR_URL: &str = "https://codexradar.com/";

pub async fn fetch_iq() -> anyhow::Result<Value> {
    let html = reqwest::Client::builder()
        .user_agent("Codex++ CodexRadar IQ")
        .build()?
        .get(CODEX_RADAR_URL)
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;
    Ok(iq_response_from_text(&html))
}

pub fn iq_response_from_text(text: &str) -> Value {
    let plain = strip_tags(text);
    let models = parse_iq_models(&plain);
    json!({
        "status": if models.as_object().is_some_and(|object| !object.is_empty()) { "ok" } else { "failed" },
        "updated_label": updated_label(&plain),
        "models": models
    })
}

fn parse_iq_models(text: &str) -> Value {
    let tokens = text.split_whitespace().collect::<Vec<_>>();
    let mut models = serde_json::Map::new();
    for index in 0..tokens.len().saturating_sub(1) {
        let Some((model, level)) = model_level(tokens[index]) else {
            continue;
        };
        let Some(iq) = leading_number(tokens[index + 1]) else {
            continue;
        };
        models
            .entry(model.to_string())
            .or_insert_with(|| json!({}))
            .as_object_mut()
            .unwrap()
            .insert(level.to_string(), json!(iq));
    }
    Value::Object(models)
}

fn model_level(token: &str) -> Option<(&str, &str)> {
    let (model, level) = token.rsplit_once('-')?;
    if !model.starts_with("GPT-") {
        return None;
    }
    match level {
        "xhigh" | "high" | "medium" | "low" => Some((model, level)),
        _ => None,
    }
}

fn leading_number(token: &str) -> Option<f64> {
    let number = token
        .trim_start_matches('*')
        .chars()
        .take_while(|ch| ch.is_ascii_digit() || *ch == '.')
        .collect::<String>();
    if number.is_empty() {
        return None;
    }
    number.parse().ok()
}

fn strip_tags(html: &str) -> String {
    let mut text = String::with_capacity(html.len());
    let mut in_tag = false;
    for ch in html.chars() {
        match ch {
            '<' => {
                in_tag = true;
                text.push(' ');
            }
            '>' => {
                in_tag = false;
                text.push(' ');
            }
            _ if !in_tag => text.push(ch),
            _ => {}
        }
    }
    text
}

fn updated_label(text: &str) -> String {
    let Some(start) = text.find("降智雷达") else {
        return String::new();
    };
    let tail = &text[start..text.len().min(start + 200)];
    let Some(end) = tail.find("更新") else {
        return String::new();
    };
    tail[..end + "更新".len()]
        .split_whitespace()
        .collect::<Vec<_>>()
        .join("")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_codexradar_iq_values_from_html() {
        let parsed = iq_response_from_text(
            r#"
            <h2>降智雷达 7月6日13:41更新</h2>
            <div>GPT-5.5-xhigh</div><strong>90.0</strong>$33.1
            <div>GPT-5.5-high</div><strong>120.0</strong>$23.5
            <div>GPT-5.5-medium</div><strong>90.0</strong>$18.6
            <div>GPT-5.5-low</div><strong>75.0</strong>$12.5
            <div>GPT-5.4-high</div><strong>75.0</strong>$13.0
            "#,
        );
        assert_eq!(parsed["status"], "ok");
        assert_eq!(parsed["models"]["GPT-5.5"]["high"], 120.0);
        assert_eq!(parsed["models"]["GPT-5.5"]["low"], 75.0);
        assert_eq!(parsed["models"]["GPT-5.4"]["high"], 75.0);
    }
}
