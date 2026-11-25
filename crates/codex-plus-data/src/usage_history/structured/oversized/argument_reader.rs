use serde_json::{Value, json};
use std::io::{self, BufRead, Read};

pub(super) fn receipt_text(input: &mut dyn BufRead, skill: bool) -> io::Result<Value> {
    let mut evidence = String::new();
    let mut header = true;
    let mut first = true;
    let mut heading = false;
    let mut wall = None;
    loop {
        if first || !header {
            skip_delimiters(input, false)?;
        }
        // The longest receipt marker fits in this probe. Read the remaining line only for its value.
        let mut line = Vec::new();
        if input.take(48).read_until(b'\n', &mut line)? == 0 {
            break;
        }
        let mut complete = line.ends_with(b"\n") || line.len() < 48;
        let sample = String::from_utf8_lossy(&line);
        let sample = sample.trim_end();
        let metadata = header
            .then(|| {
                [
                    "Wall time: ",
                    "Process running with session ID ",
                    "Process exited with code ",
                ]
                .into_iter()
                .find(|prefix| sample.starts_with(prefix))
            })
            .flatten();
        let skill_name = skill && sample.trim_start().starts_with("name:");
        if let Some(marker) = metadata {
            let mut tail = std::io::Cursor::new(&line[marker.len()..]).chain(&mut *input);
            let mut number = Vec::new();
            let valid = loop {
                let chunk = tail.fill_buf()?;
                let count = chunk
                    .iter()
                    .take_while(|b| {
                        b.is_ascii_digit() || matches!(b, b'.' | b'+' | b'-' | b'e' | b'E')
                    })
                    .count();
                number.extend_from_slice(&chunk[..count]);
                if let Some(&end) = chunk.get(count) {
                    tail.consume(count);
                    break end.is_ascii_whitespace();
                }
                if chunk.is_empty() {
                    break true;
                }
                tail.consume(count);
            };
            if valid && let Ok(number) = std::str::from_utf8(&number) {
                match marker {
                    "Wall time: " => {
                        let mut unit = [0; 8];
                        let mut read = 0;
                        while read < unit.len() {
                            let bytes = tail.fill_buf()?;
                            if bytes.is_empty() || bytes[0] == b'\n' {
                                break;
                            }
                            unit[read] = bytes[0];
                            tail.consume(1);
                            read += 1;
                        }
                        if unit == *b" seconds"
                            && let Ok(seconds) = number.parse::<f64>()
                            && seconds.is_finite()
                            && seconds >= 0.0
                        {
                            wall = Some(seconds);
                        }
                    }
                    "Process running with session ID " => {
                        if number.bytes().all(|b| b.is_ascii_digit())
                            && let Ok(id) = number.parse::<u64>()
                        {
                            if let Some(seconds) = wall { evidence.push_str(&format!("Wall time: {seconds} seconds\n")); }
                            evidence.push_str(&format!("{marker}{id}\n"));
                        }
                        header = false;
                    }
                    _ => {
                        if let Ok(code) = number.parse::<i32>() {
                            if let Some(seconds) = wall { evidence.push_str(&format!("Wall time: {seconds} seconds\n")); }
                            evidence.push_str(&format!("{marker}{code}\n"));
                        }
                        header = false;
                    }
                }
            } else if marker.starts_with("Process ") {
                header = false;
            }
            tail.skip_until(b'\n')?;
            complete = true;
        } else if skill_name {
            if !complete {
                input.read_until(b'\n', &mut line)?;
                complete = true;
            }
            let text = String::from_utf8_lossy(&line);
            let text = text.trim();
            let name = text["name:".len()..].trim().trim_matches(['\'', '"']);
            if !name.is_empty()
                && name
                    .chars()
                    .all(|c| c.is_alphanumeric() || "._-:".contains(c))
            {
                evidence.push_str("name: ");
                evidence.push_str(name);
                evidence.push('\n');
            }
            header = false;
        } else if header
            && (sample.starts_with("Chunk ID:") || sample.starts_with("Original token count:"))
        {
            // Header framing carries no activity data.
        } else {
            if first && super::super::tool_receipts::failed_text(sample) {
                evidence.push_str("Error: tool execution failed\n");
            } else if first
                && [
                    "status is unknown",
                    "outcome is unknown",
                    "did not complete",
                ]
                .iter()
                .any(|s| sample.to_ascii_lowercase().starts_with(s))
            {
                return Ok(json!({"status":"unknown"}));
            }
            heading |= skill
                && (sample.starts_with("# ")
                    || sample.starts_with("## ")
                    || complete && sample == "---");
            header = false;
        }
        if !header && !skill {
            break;
        }
        if !complete {
            input.skip_until(b'\n')?;
        }
        first = false;
    }
    if heading {
        evidence.push_str("# Skill content observed");
    }
    if evidence.ends_with('\n') {
        evidence.pop();
    }
    Ok(Value::String(evidence))
}

pub(super) fn skip_delimiters(input: &mut dyn BufRead, colon: bool) -> io::Result<()> {
    loop {
        let buffer = input.fill_buf()?;
        let len = buffer
            .iter()
            .take_while(|&&b| b.is_ascii_whitespace() || colon && matches!(b, b':' | b','))
            .count();
        let finished = len < buffer.len() || buffer.is_empty();
        input.consume(len);
        if finished {
            return Ok(());
        }
    }
}

// 原生 function_call 的 arguments 通常是一层 JSON 字符串。分块解码后，
// 内层 serde 可以直接跳过 chars、补丁正文等字段，不需要复制整个外层字符串。
pub(super) struct JsonStringContents<R> {
    input: R,
    escape: std::io::Cursor<Vec<u8>>,
    done: bool,
}

impl<R: BufRead> JsonStringContents<R> {
    pub(super) fn new(input: R) -> Self {
        Self {
            input,
            escape: std::io::Cursor::default(),
            done: false,
        }
    }
}

impl<R: BufRead> Read for JsonStringContents<R> {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        if output.is_empty() || self.done {
            return Ok(0);
        }
        let mut n = self.escape.read(output)?;
        while n < output.len() {
            let buffer = self.input.fill_buf()?;
            let Some(&first) = buffer.first() else {
                return Err(io::ErrorKind::UnexpectedEof.into());
            };
            match first {
                b'"' => {
                    self.input.consume(1);
                    self.done = true;
                    break;
                }
                b'\\' => {
                    let mut escaped = [b'"'; 14];
                    self.input.read_exact(&mut escaped[1..3])?;
                    let mut end = 3;
                    if escaped[2] == b'u' {
                        self.input.read_exact(&mut escaped[3..7])?;
                        end = 7;
                        if escaped[3].eq_ignore_ascii_case(&b'd')
                            && matches!(escaped[4].to_ascii_lowercase(), b'8'..=b'9' | b'a'..=b'b')
                        {
                            self.input.read_exact(&mut escaped[7..13])?;
                            end = 13;
                        }
                    }
                    let decoded: String = serde_json::from_slice(&escaped[..=end])?;
                    self.escape = std::io::Cursor::new(decoded.into_bytes());
                    n += self.escape.read(&mut output[n..])?;
                }
                _ => {
                    let take = buffer.len().min(output.len() - n);
                    let take = buffer[..take]
                        .iter()
                        .position(|&b| b == b'\\' || b == b'"')
                        .unwrap_or(take);
                    output[n..n + take].copy_from_slice(&buffer[..take]);
                    self.input.consume(take);
                    n += take;
                }
            }
        }
        Ok(n)
    }
}
