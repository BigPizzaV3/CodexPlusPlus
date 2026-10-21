//! 原生历史可透明压缩为 `.jsonl.zst`，数据库仍可能保存未压缩的逻辑路径。
use anyhow::Context;
use std::fs::File;
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};

pub(crate) fn is_compressed(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.ends_with(".jsonl.zst"))
}

pub(crate) fn is_rollout(path: &Path) -> bool {
    logical_stem(path).is_some()
}

pub(crate) fn logical_stem(path: &Path) -> Option<&str> {
    let name = path.file_name()?.to_str()?;
    name.strip_suffix(".jsonl.zst")
        .or_else(|| name.strip_suffix(".jsonl"))
}

/// 只回退同一个逻辑文件，不能按相似文件名猜测其他历史。
pub(crate) fn resolve(path: &Path) -> Option<PathBuf> {
    if path.is_file() {
        return Some(path.to_path_buf());
    }
    if path.extension().is_some_and(|extension| extension == "jsonl") {
        let mut compressed = path.as_os_str().to_os_string();
        compressed.push(".zst");
        let compressed = PathBuf::from(compressed);
        if compressed.is_file() {
            return Some(compressed);
        }
    }
    None
}

pub(crate) fn decode_reader<'a>(
    path: &Path,
    reader: impl Read + 'a,
) -> anyhow::Result<Box<dyn Read + 'a>> {
    if is_compressed(path) {
        Ok(Box::new(
            zstd::stream::read::Decoder::new(reader)
                .with_context(|| format!("无法解压原生历史：{}", path.display()))?,
        ))
    } else {
        Ok(Box::new(reader))
    }
}

pub(crate) fn open(path: &Path) -> anyhow::Result<Box<dyn Read>> {
    let file = File::open(path)
        .with_context(|| format!("无法打开原生历史：{}", path.display()))?;
    decode_reader(path, BufReader::new(file))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn compressed_native_source_resolves_and_streams_without_changing_bytes() {
        let home = tempdir().unwrap();
        let logical = home.path().join("rollout-parent_child.jsonl");
        let compressed = home.path().join("rollout-parent_child.jsonl.zst");
        let plain = b"{\"type\":\"session_meta\"}\n";
        let bytes = zstd::stream::encode_all(&plain[..], 1).unwrap();
        fs::write(&compressed, &bytes).unwrap();
        assert_eq!(resolve(&logical), Some(compressed.clone()));
        assert_eq!(logical_stem(&compressed), Some("rollout-parent_child"));
        assert!(is_rollout(&compressed));
        assert!(!is_rollout(&home.path().join("other.zst")));
        let mut decoded = Vec::new();
        open(&compressed).unwrap().read_to_end(&mut decoded).unwrap();
        assert_eq!(decoded, plain);
        assert_eq!(fs::read(&compressed).unwrap(), bytes);
        fs::write(&logical, b"current").unwrap();
        assert_eq!(resolve(&logical), Some(logical.clone()));
    }

    #[test]
    fn malformed_compressed_source_is_not_treated_as_empty_history() {
        let home = tempdir().unwrap();
        let compressed = home.path().join("rollout.jsonl.zst");
        fs::write(&compressed, b"not a zstd stream").unwrap();
        let result = open(&compressed)
            .and_then(|mut reader| reader.read_to_end(&mut Vec::new()).map_err(Into::into));
        assert!(result.is_err());
    }
}
