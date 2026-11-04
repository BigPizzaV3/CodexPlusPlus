//! 仅判断原生日志前缀的事件类型，正文中的同名字符串不参与分类。
struct UsagePrefix<'a> {
    payload: bool,
    result: &'a std::cell::Cell<Option<bool>>,
}
impl<'de> serde::de::DeserializeSeed<'de> for UsagePrefix<'_> {
    type Value = bool;
    fn deserialize<D: serde::Deserializer<'de>>(self, deserializer: D) -> Result<bool, D::Error> {
        deserializer.deserialize_map(self)
    }
}
impl<'de> serde::de::Visitor<'de> for UsagePrefix<'_> {
    type Value = bool;
    fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("rollout event")
    }
    fn visit_map<M: serde::de::MapAccess<'de>>(self, mut map: M) -> Result<bool, M::Error> {
        let mut event_msg = false;
        while let Some(key) = map.next_key::<String>()? {
            if key == "type" {
                let kind = map.next_value::<String>()?;
                if self.payload {
                    let relevant = kind.starts_with("thread_settings")
                        || matches!(
                            kind.as_str(),
                            "token_count"
                                | "task_started"
                                | "turn_started"
                                | "task_complete"
                                | "turn_completed"
                                | "task_failed"
                                | "turn_aborted"
                                | "task_aborted"
                                | "error"
                                | "stream_error"
                        );
                    self.result.set(Some(relevant));
                    return Ok(relevant);
                }
                match kind.as_str() {
                    "session_meta" | "turn_context" => {
                        self.result.set(Some(true));
                        return Ok(true);
                    }
                    "event_msg" => event_msg = true,
                    _ => {
                        self.result.set(Some(false));
                        return Ok(false);
                    }
                }
            } else if key == "payload" && event_msg {
                return map.next_value_seed(UsagePrefix {
                    payload: true,
                    result: self.result,
                });
            } else {
                map.next_value::<serde::de::IgnoredAny>()?;
            }
        }
        Ok(true)
    }
}
pub(super) fn may_affect_usage(prefix: &[u8]) -> bool {
    let result = std::cell::Cell::new(None);
    // 前缀可能没有闭合括号；已确认的类型独立保存，不依赖完整 JSON 解析成功。
    let _ = serde::Deserializer::deserialize_map(
        &mut serde_json::Deserializer::from_slice(prefix),
        UsagePrefix {
            payload: false,
            result: &result,
        },
    );
    result.get().unwrap_or(true)
}
