//! 大图压缩：请求体里的 base64 data URL 图片超过阈值时，缩放并转成 JPEG，
//! 避免撑爆上游的请求体上限（413）。默认关闭，按供应商开关（`imageCompress`）。
//!
//! 缩放/转码用 macOS 自带的 `sips`（系统组件，无额外依赖）；其他平台暂不可用，
//! 保持原样放行。压缩失败（sips 出错、格式不支持、超时）时同样原样放行，
//! 不阻断请求。并发安全：每次压缩使用进程内唯一的临时路径。

use base64::Engine as _;
use serde_json::Value;

/// 图片 data URL 长度阈值：≥ 该值才压缩（与常见 413 门槛留足余量）。
pub const IMAGE_COMPRESS_MIN_BYTES: usize = 256 * 1024;
/// 缩放后最长边（像素）。
pub const IMAGE_COMPRESS_MAX_DIMENSION: u32 = 1600;
/// JPEG 质量（sips formatOptions，1-100）。
pub const IMAGE_COMPRESS_JPEG_QUALITY: u32 = 60;
/// 单张图压缩的墙钟上限；超时视为失败，原样放行。
const COMPRESS_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

/// 对 Responses / Chat 请求的消息数组做压缩：收集 `input_image` / `image_url`
/// 部件里的 base64 图片 data URL，超过阈值就替换为压缩结果。
/// 返回发生替换的数量（用于诊断日志）。
pub async fn compress_image_data_urls(messages: &mut [Value]) -> usize {
    // 先同步收集所有可压缩目标的可变引用，再逐个异步压缩，避免 async 递归。
    let mut targets: Vec<&mut String> = Vec::new();
    for message in messages.iter_mut() {
        collect_targets(message, &mut targets);
    }
    let mut replaced = 0;
    for url in targets {
        if let Some(compressed) = compress_data_url(url).await {
            *url = compressed;
            replaced += 1;
        }
    }
    replaced
}

/// 递归收集 image_url 子树里的 base64 图片 data URL 字符串的可变引用。
fn collect_targets<'a>(value: &'a mut Value, out: &mut Vec<&'a mut String>) {
    match value {
        Value::Object(map) => {
            for (key, child) in map.iter_mut() {
                // image_url 子树是图片的合法归宿；其余位置出现的 data URL 属于
                // 文本泄漏，由 guard_inline_image_data_urls 处理，这里不碰。
                if key == "image_url" {
                    collect_image_url_targets(child, out);
                } else {
                    collect_targets(child, out);
                }
            }
        }
        Value::Array(items) => {
            for item in items {
                collect_targets(item, out);
            }
        }
        _ => {}
    }
}

fn collect_image_url_targets<'a>(value: &'a mut Value, out: &mut Vec<&'a mut String>) {
    match value {
        // Responses 形态：`"image_url": "data:..."`
        Value::String(url) => {
            if is_compressible(url) {
                out.push(url);
            }
        }
        // Chat 形态：`"image_url": { "url": "data:..." }`
        Value::Object(map) => {
            if let Some(Value::String(url)) = map.get_mut("url") {
                if is_compressible(url) {
                    out.push(url);
                }
            }
        }
        _ => {}
    }
}

/// 是否为可能可压缩的 base64 图片 data URL（超过阈值、支持的格式）。
fn is_compressible(url: &str) -> bool {
    if url.len() < IMAGE_COMPRESS_MIN_BYTES {
        return false;
    }
    let Some((meta, _)) = url.split_once(";base64,") else {
        return false;
    };
    matches!(
        meta.strip_prefix("data:image/"),
        Some("png" | "jpeg" | "jpg" | "webp")
    )
}

/// 压缩单个 data URL；失败返回 None（原样放行）。
async fn compress_data_url(url: &str) -> Option<String> {
    let (_, payload) = url.split_once(";base64,")?;
    let format = url
        .strip_prefix("data:image/")?
        .split(';')
        .next()?
        .to_string();
    let format = match format.as_str() {
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

    let compressed = compress_with_sips(bytes, format).await?;
    let compressed_url = format!(
        "data:image/jpeg;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(compressed)
    );
    // 只在确实更小时才替换；个别图（高熵纹理）转 JPEG 后可能不降反升。
    (compressed_url.len() < url.len()).then_some(compressed_url)
}

/// 调 macOS `sips` 缩放并转 JPEG（spawn_blocking + 超时，避免阻塞异步线程）。
/// 非 macOS 或 sips 失败/超时返回 None。
async fn compress_with_sips(bytes: Vec<u8>, format: &'static str) -> Option<Vec<u8>> {
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (bytes, format);
        None
    }
    #[cfg(target_os = "macos")]
    {
        // 每次调用唯一目录：并发请求（多会话/子代理）不会互串文件。
        let dir = std::env::temp_dir().join(format!(
            "codexpp-img-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        let input = dir.join(format!("in.{format}"));
        let output = dir.join("out.jpg");
        let cleanup_dir = dir.clone();
        let run = tokio::task::spawn_blocking(move || -> Option<Vec<u8>> {
            use std::io::Write as _;
            std::fs::create_dir_all(&dir).ok()?;
            {
                let mut file = std::fs::File::create(&input).ok()?;
                file.write_all(&bytes).ok()?;
            }
            let status = std::process::Command::new("/usr/bin/sips")
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
            let compressed = match status {
                Ok(out) if out.status.success() => std::fs::read(&output).ok(),
                _ => None,
            };
            let _ = std::fs::remove_dir_all(&dir);
            compressed.filter(|data| !data.is_empty())
        });
        match tokio::time::timeout(COMPRESS_TIMEOUT, run).await {
            Ok(Ok(compressed)) => compressed,
            _ => {
                // 超时/任务失败：清理临时目录（任务可能仍持着旧路径，删除失败无妨），
                // 图片原样放行。
                let _ = tokio::fs::remove_dir_all(&cleanup_dir).await;
                None
            }
        }
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

    #[tokio::test]
    async fn skips_small_images() {
        let mut messages = json!([
            {"content": [
                {"type": "input_text", "text": "hi"},
                {"type": "input_image", "image_url": tiny_png_data_url()}
            ]}
        ]);
        let arr = messages.as_array_mut().unwrap();
        assert_eq!(compress_image_data_urls(arr).await, 0);
        assert!(messages.to_string().contains("iVBORw0KGgo"));
    }

    #[tokio::test]
    async fn skips_non_image_and_non_base64() {
        let mut messages = json!([
            {"content": [
                {"type": "image_url", "image_url": {"url": "https://example.com/a.png"}},
                {"type": "text", "text": "data:image/png;base64,!!!"}
            ]}
        ]);
        let arr = messages.as_array_mut().unwrap();
        assert_eq!(compress_image_data_urls(arr).await, 0);
    }

    #[tokio::test]
    async fn skips_below_threshold_data_urls_anywhere() {
        let mut messages = json!([
            {"content": [
                {"type": "input_image", "image_url": format!("data:image/png;base64,{}", "A".repeat(1000))}
            ]}
        ]);
        let arr = messages.as_array_mut().unwrap();
        assert_eq!(compress_image_data_urls(arr).await, 0);
    }

    #[tokio::test]
    async fn keeps_invalid_large_images_untouched() {
        // 400KB 的伪 payload：超过阈值；base64 能解码但不是合法图片，
        // sips 失败 → 保持原样（安全回退）。
        let big = format!("data:image/png;base64,{}", "A".repeat(400_000));
        let mut messages = json!([
            {"content": [{"type": "input_image", "image_url": big}]}
        ]);
        let arr = messages.as_array_mut().unwrap();
        assert_eq!(compress_image_data_urls(arr).await, 0);
    }

    #[cfg(target_os = "macos")]
    #[tokio::test]
    async fn compresses_real_large_png_on_macos() {
        // 生成一张 2000x1500 的渐变 PNG（data URL 必然超过 256KB 阈值），
        // 压缩后应是合法的 data:image/jpeg，且长度降到阈值以下。
        let png = make_solid_png(2000, 1500);
        let url = format!(
            "data:image/png;base64,{}",
            base64::engine::general_purpose::STANDARD.encode(png)
        );
        assert!(url.len() >= IMAGE_COMPRESS_MIN_BYTES);
        let mut messages = json!([
            {"content": [{"type": "input_image", "image_url": url}]}
        ]);
        let arr = messages.as_array_mut().unwrap();
        assert_eq!(compress_image_data_urls(arr).await, 1);
        let out = messages[0]["content"][0]["image_url"].as_str().unwrap();
        assert!(
            out.starts_with("data:image/jpeg;base64,"),
            "got: {}",
            &out[..48]
        );
        assert!(
            out.len() < IMAGE_COMPRESS_MIN_BYTES,
            "compressed should shrink below threshold"
        );
    }

    #[cfg(target_os = "macos")]
    fn make_solid_png(width: u32, height: u32) -> Vec<u8> {
        // 手写最小 PNG：IHDR + IDAT（zlib）+ IEND，避免测试依赖图片库。
        use std::io::Write;
        let mut png = Vec::new();
        png.extend_from_slice(b"\x89PNG\r\n\x1a\n");
        let mut chunk = |kind: &[u8; 4], data: &[u8], out: &mut Vec<u8>| {
            out.extend_from_slice(&(data.len() as u32).to_be_bytes());
            out.extend_from_slice(kind);
            out.extend_from_slice(data);
            let mut crc = 0xFFFF_FFFFu32;
            for byte in kind.iter().chain(data.iter()) {
                crc ^= *byte as u32;
                for _ in 0..8 {
                    crc = if crc & 1 != 0 {
                        (crc >> 1) ^ 0xEDB8_8320
                    } else {
                        crc >> 1
                    };
                }
            }
            out.extend_from_slice(&(crc ^ 0xFFFF_FFFF).to_be_bytes());
        };
        let mut ihdr = Vec::new();
        ihdr.extend_from_slice(&width.to_be_bytes());
        ihdr.extend_from_slice(&height.to_be_bytes());
        ihdr.extend_from_slice(&[8, 2, 0, 0, 0]); // 8bit RGB
        chunk(b"IHDR", &ihdr, &mut png);
        // 每行：filter 0 + RGB 像素（渐变，zlib 可压缩）。
        let mut raw = Vec::with_capacity(height as usize * (1 + width as usize * 3));
        for y in 0..height {
            raw.push(0u8);
            for x in 0..width {
                raw.extend_from_slice(&[(x % 256) as u8, (y % 256) as u8, 128u8]);
            }
        }
        let mut encoder =
            flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(&raw).unwrap();
        let idat = encoder.finish().unwrap();
        chunk(b"IDAT", &idat, &mut png);
        chunk(b"IEND", &[], &mut png);
        png
    }
}
