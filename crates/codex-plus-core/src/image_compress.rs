//! 大图压缩：请求体里的 base64 data URL 图片超过阈值时，缩放并转成 JPEG，
//! 避免撑爆上游的请求体上限（413）。默认关闭，按供应商开关（`imageCompress`）。
//!
//! 缩放/转码用 macOS 自带的 `sips`（系统组件，无额外依赖）；其他平台暂不可用，
//! 保持原样放行。压缩失败（sips 出错、格式不支持）时同样原样放行，不阻断请求。

use base64::Engine as _;
use serde_json::Value;

/// 图片 data URL 长度阈值：≥ 该值才压缩（与常见 413 门槛留足余量）。
pub const IMAGE_COMPRESS_MIN_BYTES: usize = 256 * 1024;
/// 缩放后最长边（像素）。
pub const IMAGE_COMPRESS_MAX_DIMENSION: u32 = 1600;
/// JPEG 质量（sips formatOptions，1-100）。
pub const IMAGE_COMPRESS_JPEG_QUALITY: u32 = 60;

/// 遍历 body 中所有字符串字段，把超过阈值的图片 data URL 原地替换为压缩结果。
/// 返回发生替换的数量（用于诊断日志）。
pub fn compress_image_data_urls(value: &mut Value) -> usize {
    let mut replaced = 0;
    compress_walk(value, &mut replaced);
    replaced
}

fn compress_walk(value: &mut Value, replaced: &mut usize) {
    match value {
        Value::Object(map) => {
            for (key, child) in map.iter_mut() {
                // image_url 子树是图片的合法归宿；其余位置出现的 data URL 属于
                // 文本泄漏，由 guard_inline_image_data_urls 处理，这里不碰。
                if key == "image_url" {
                    compress_image_url_field(child, replaced);
                } else {
                    compress_walk(child, replaced);
                }
            }
        }
        Value::Array(items) => {
            for item in items {
                compress_walk(item, replaced);
            }
        }
        _ => {}
    }
}

fn compress_image_url_field(value: &mut Value, replaced: &mut usize) {
    match value {
        // Responses 形态：`"image_url": "data:..."`
        Value::String(url) => {
            if let Some(compressed) = compress_data_url(url) {
                *value = Value::String(compressed);
                *replaced += 1;
            }
        }
        // Chat 形态：`"image_url": { "url": "data:..." }`
        Value::Object(map) => {
            if let Some(url) = map.get_mut("url") {
                if let Value::String(text) = url {
                    if let Some(compressed) = compress_data_url(text) {
                        *url = Value::String(compressed);
                        *replaced += 1;
                    }
                }
            }
        }
        _ => {}
    }
}

/// 判断是否为 base64 图片 data URL，超过阈值则压缩；否则返回 None。
fn compress_data_url(url: &str) -> Option<String> {
    if url.len() < IMAGE_COMPRESS_MIN_BYTES {
        return None;
    }
    let (meta, payload) = url.split_once(";base64,")?;
    let format = meta.strip_prefix("data:image/")?;
    let format = match format {
        "png" => "png",
        "jpeg" | "jpg" => "jpg",
        "webp" => "webp",
        _ => return None,
    };
    // base64 合法性粗检：解码失败（非法字符/长度）就不动它。
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(payload)
        .ok()?;
    if bytes.is_empty() {
        return None;
    }

    let compressed = compress_with_sips(&bytes, format)?;
    Some(format!(
        "data:image/jpeg;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(compressed)
    ))
}

/// 调 macOS `sips` 缩放并转 JPEG。非 macOS 或 sips 失败时返回 None。
fn compress_with_sips(bytes: &[u8], format: &str) -> Option<Vec<u8>> {
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (bytes, format);
        None
    }
    #[cfg(target_os = "macos")]
    {
        use std::io::Write as _;
        let dir = std::env::temp_dir().join(format!("codexpp-img-{}", std::process::id()));
        std::fs::create_dir_all(&dir).ok()?;
        let input = dir.join(format!("in.{format}"));
        let output = dir.join("out.jpg");
        {
            let mut file = std::fs::File::create(&input).ok()?;
            file.write_all(bytes).ok()?;
        }
        let result = std::process::Command::new("/usr/bin/sips")
            .args([
                "-Z",
                &IMAGE_COMPRESS_MAX_DIMENSION.to_string(),
                "-s",
                "format",
                "jpeg",
                "-s",
                "formatOptions",
                &IMAGE_COMPRESS_JPEG_QUALITY.to_string(),
            ])
            .arg(&input)
            .arg("--out")
            .arg(&output)
            .output();
        let compressed = match result {
            Ok(out) if out.status.success() => std::fs::read(&output).ok(),
            _ => None,
        };
        let _ = std::fs::remove_file(&input);
        let _ = std::fs::remove_file(&output);
        let _ = std::fs::remove_dir(&dir);
        compressed.filter(|data| !data.is_empty())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn tiny_png_data_url() -> String {
        // 1x1 红色 PNG
        let png = [
            0x89u8, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0, 0, 0, 0x0D, 0x49, 0x48, 0x44,
            0x52, 0, 0, 0, 1, 0, 0, 0, 1, 8, 2, 0, 0, 0, 0x90, 0x77, 0x53, 0xDE, 0, 0, 0, 0x0C,
            0x49, 0x44, 0x41, 0x54, 0x08, 0xD7, 0x63, 0xF8, 0xCF, 0xC0, 0, 0, 3, 1, 1, 0, 0x18,
            0x2D, 0x4C, 0x59, 0x0, 0, 0, 0, 0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
        ];
        format!(
            "data:image/png;base64,{}",
            base64::engine::general_purpose::STANDARD.encode(png)
        )
    }

    #[test]
    fn skips_small_images() {
        let mut body = json!({
            "input": [{"content": [
                {"type": "input_text", "text": "hi"},
                {"type": "input_image", "image_url": tiny_png_data_url()}
            ]}]
        });
        assert_eq!(compress_image_data_urls(&mut body), 0);
        assert!(body.to_string().contains("iVBORw0KGgo"));
    }

    #[test]
    fn skips_non_image_and_non_base64() {
        let mut body = json!({
            "messages": [{"content": [
                {"type": "image_url", "image_url": {"url": "https://example.com/a.png"}},
                {"type": "text", "text": "data:image/png;base64,!!!"}
            ]}]
        });
        assert_eq!(compress_image_data_urls(&mut body), 0);
    }

    #[test]
    fn skips_below_threshold_data_urls_anywhere() {
        let mut body = json!({"input": [{"content": [
            {"type": "input_image", "image_url": format!("data:image/png;base64,{}", "A".repeat(1000))}
        ]}]});
        assert_eq!(compress_image_data_urls(&mut body), 0);
    }

    #[test]
    fn compresses_large_image_url_string_form() {
        // 400KB 的伪 payload：超过阈值；sips 会因非法图片而失败 → 保持原样。
        // 这里验证"超大但解码失败/压缩失败时不改写"的安全行为。
        let big = format!("data:image/png;base64,{}", "A".repeat(400_000));
        let mut body = json!({"input": [{"content": [
            {"type": "input_image", "image_url": big}
        ]}]});
        assert_eq!(compress_image_data_urls(&mut body), 0);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn compresses_real_large_png_on_macos() {
        // 生成一张 2000x1500 的纯色 PNG（必然超过 256KB 的 data URL 阈值），
        // 压缩后应是合法的 data:image/jpeg。
        let png = make_solid_png(2000, 1500);
        let url = format!(
            "data:image/png;base64,{}",
            base64::engine::general_purpose::STANDARD.encode(png)
        );
        assert!(url.len() >= IMAGE_COMPRESS_MIN_BYTES);
        let mut body = json!({"input": [{"content": [
            {"type": "input_image", "image_url": url}
        ]}]});
        assert_eq!(compress_image_data_urls(&mut body), 1);
        let out = body["input"][0]["content"][1]["image_url"]
            .as_str()
            .unwrap();
        assert!(out.starts_with("data:image/jpeg;base64,"), "got: {}", &out[..48]);
        assert!(out.len() < IMAGE_COMPRESS_MIN_BYTES, "compressed should shrink below threshold");
    }

    #[cfg(target_os = "macos")]
    fn make_solid_png(width: u32, height: u32) -> Vec<u8> {
        // 手写最小 PNG：IHDR + IDAT（zlib 存储块）+ IEND，避免测试依赖图片库。
        use std::io::Write;
        let mut png = Vec::new();
        png.extend_from_slice(b"\x89PNG\r\n\x1a\n");
        let mut chunk = |kind: &[u8; 4], data: &[u8], out: &mut Vec<u8>| {
            out.extend_from_slice(&(data.len() as u32).to_be_bytes());
            out.extend_from_slice(kind);
            out.extend_from_slice(data);
            let mut crc = crc32(kind);
            crc = crc32_update(crc, data);
            out.extend_from_slice(&crc.to_be_bytes());
        };
        let mut ihdr = Vec::new();
        ihdr.extend_from_slice(&width.to_be_bytes());
        ihdr.extend_from_slice(&height.to_be_bytes());
        ihdr.extend_from_slice(&[8, 2, 0, 0, 0]);
        chunk(b"IHDR", &ihdr, &mut png);
        // 每行：filter 0 + RGB 像素（渐变，zlib 可压缩）。
        let mut raw = Vec::with_capacity(height as usize * (1 + width as usize * 3));
        for y in 0..height {
            raw.push(0u8);
            for x in 0..width {
                raw.extend_from_slice(&[(x % 256) as u8, (y % 256) as u8, 128u8]);
            }
        }
        let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(&raw).unwrap();
        let idat = encoder.finish().unwrap();
        chunk(b"IDAT", &idat, &mut png);
        chunk(b"IEND", &[], &mut png);
        png
    }

    #[cfg(target_os = "macos")]
    fn crc32(data: &[u8]) -> u32 {
        crc32_update(0xFFFFFFFF, data) ^ 0xFFFFFFFF
    }

    #[cfg(target_os = "macos")]
    fn crc32_update(mut crc: u32, data: &[u8]) -> u32 {
        // 标准 CRC-32（IEEE），无查表实现，测试够用。
        for byte in data {
            crc ^= *byte as u32;
            for _ in 0..8 {
                crc = if crc & 1 != 0 { (crc >> 1) ^ 0xEDB8_8320 } else { crc >> 1 };
            }
        }
        crc
    }
}
