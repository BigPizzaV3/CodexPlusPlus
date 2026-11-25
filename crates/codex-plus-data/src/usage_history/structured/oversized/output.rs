use super::{
    FieldPrefix, PrefixReader,
    argument_reader::{JsonStringContents, receipt_text, skip_delimiters},
};
use serde::de::{IgnoredAny, MapAccess, SeqAccess, Visitor};
use serde_json::{Map, Value, json};
use std::io::{self, BufRead, BufReader, Read, Seek, SeekFrom};

// Values may precede their type or contain an entire document. Locate first, then read
// just the fields that the receipt format allows to contribute evidence.
type Span = (u64, u64);
pub(super) enum Fields {
    Object(std::collections::BTreeMap<String, Span>),
    Array(Vec<Span>),
}

impl<'de> Visitor<'de> for &FieldPrefix {
    type Value = Fields;
    fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("a structured tool result")
    }
    fn visit_map<M: MapAccess<'de>>(self, mut map: M) -> Result<Fields, M::Error> {
        let mut fields = std::collections::BTreeMap::new();
        while let Some(key) = map.next_key::<String>()? {
            let start = self.offset.get();
            map.next_value::<IgnoredAny>()?;
            if matches!(
                key.as_str(),
                "success"
                    | "isError"
                    | "error"
                    | "exit_code"
                    | "exitCode"
                    | "status"
                    | "type"
                    | "text"
                    | "content"
            ) {
                fields.insert(key, (start, self.offset.get() - start));
            }
        }
        Ok(Fields::Object(fields))
    }
    fn visit_seq<S: SeqAccess<'de>>(self, mut seq: S) -> Result<Fields, S::Error> {
        let mut fields = Vec::new();
        loop {
            let start = self.offset.get();
            self.first.set(None);
            self.probe.set(true);
            let next = seq.next_element::<IgnoredAny>()?;
            self.probe.set(false);
            if next.is_none() {
                break;
            }
            if matches!(self.first.get(), Some(b'[' | b'{' | b'"')) {
                fields.push((start, self.offset.get() - start));
            }
        }
        Ok(Fields::Array(fields))
    }
}

fn field<R: Read + Seek>(
    source: &mut R,
    (start, len): Span,
) -> io::Result<BufReader<io::Take<&mut R>>> {
    source.seek(SeekFrom::Start(start))?;
    let mut input = BufReader::new(source.take(len));
    skip_delimiters(&mut input, true)?;
    Ok(input)
}

pub(super) fn project<R: Read + Seek>(
    source: &mut R,
    start: u64,
    len: u64,
    skill: bool,
) -> io::Result<Value> {
    let mut input = field(source, (start, len))?;
    let shape = input.fill_buf()?.first().copied();
    if shape == Some(b'"') {
        input.consume(1);
        let mut text = BufReader::new(JsonStringContents::new(input));
        skip_delimiters(&mut text, false)?;
        if matches!(text.fill_buf()?.first(), Some(b'{' | b'[')) {
            let mut decoded = tempfile::tempfile()?;
            let length = io::copy(&mut text, &mut decoded)?;
            match project(&mut decoded, 0, length, skill) {
                Ok(value) => return Ok(value),
                Err(error)
                    if matches!(
                        error.kind(),
                        io::ErrorKind::InvalidData | io::ErrorKind::UnexpectedEof
                    ) =>
                {
                    decoded.rewind()?;
                    return receipt_text(&mut BufReader::new(decoded), skill);
                }
                Err(error) => return Err(error),
            }
        }
        return receipt_text(&mut text, skill);
    }
    if !matches!(shape, Some(b'{' | b'[')) {
        return Ok(Value::Null);
    }
    let base = start + len - input.get_ref().limit() - input.buffer().len() as u64;
    let positions = FieldPrefix::default();
    let mut de = serde_json::Deserializer::from_reader(PrefixReader(&mut input, &positions));
    let fields = serde::Deserializer::deserialize_any(&mut de, &positions)?;
    match fields {
        Fields::Array(spans) => {
            let mut results = Vec::new();
            for (at, length) in spans {
                let part = project(source, base + at, length, skill)?;
                if !part.is_null() && part.as_str() != Some("") {
                    results.push(part);
                }
            }
            Ok(Value::Array(results))
        }
        Fields::Object(mut fields) => {
            let mut result = Map::new();
            for key in [
                "isError",
                "success",
                "error",
                "exit_code",
                "exitCode",
                "status",
                "type",
            ] {
                let Some((offset, length)) = fields.remove(key) else {
                    continue;
                };
                let mut input = field(source, (base + offset, length))?;
                let first = input.fill_buf()?.first().copied();
                let value = match (key, first) {
                    ("isError" | "success", Some(b't' | b'f')) => json!(first == Some(b't')),
                    ("error", Some(b'"' | b'{' | b'[' | b't' | b'0'..=b'9' | b'-')) => {
                        Value::Bool(true)
                    }
                    ("exit_code" | "exitCode", Some(b'0'..=b'9' | b'-')) => {
                        let value: serde_json::Result<i64> = serde::Deserialize::deserialize(
                            &mut serde_json::Deserializer::from_reader(input),
                        );
                        match value {
                            Ok(code) => Value::from(code),
                            Err(error) if error.is_io() => return Err(error.into()),
                            Err(_) => continue,
                        }
                    }
                    ("status" | "type", Some(b'"')) => {
                        input.consume(1);
                        let allowed: &[&str] = if key == "type" {
                            &["text", "input_text", "output_text"]
                        } else {
                            &[
                                "success",
                                "completed",
                                "failed",
                                "error",
                                "cancelled",
                                "canceled",
                                "interrupted",
                                "unknown",
                                "pending",
                                "running",
                                "in_progress",
                            ]
                        };
                        let mut keyword = Vec::new();
                        JsonStringContents::new(input)
                            .take(allowed.iter().map(|s| s.len()).max().unwrap() as u64 + 1)
                            .read_to_end(&mut keyword)?;
                        let Some(word) = allowed.iter().find(|word| word.as_bytes() == keyword)
                        else {
                            continue;
                        };
                        json!(word)
                    }
                    _ => continue,
                };
                result.insert(key.into(), value);
            }
            if result.contains_key("type")
                && let Some((at, n)) = fields.get("text")
            {
                result.insert("text".into(), project(source, base + at, *n, skill)?);
            }
            if let Some((at, n)) = fields.get("content")
                && field(source, (base + at, *n))?.fill_buf()?.first() == Some(&b'[')
            {
                result.insert("content".into(), project(source, base + at, *n, skill)?);
            }
            Ok(Value::Object(result))
        }
    }
}
