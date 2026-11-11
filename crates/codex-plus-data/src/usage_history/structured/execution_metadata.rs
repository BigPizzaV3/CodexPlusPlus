//! 旧索引丢失的轻量分类按原位置补齐，不重扫用量或读取正文详情。
use super::{ObservationKind, parse_line};
use crate::usage_history::structured::persistent_index::{decode_source, encode_identity};
use anyhow::{Context, ensure};
use rusqlite::{Connection, params};
use std::fs::{self, File};
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

#[derive(Clone)]
pub(in crate::usage_history::structured) struct MetadataCursor {
    source: Vec<u8>,
    start: i64,
}

pub(in crate::usage_history::structured) struct MetadataRepair {
    pub(in crate::usage_history::structured) next: Option<MetadataCursor>,
    pub(in crate::usage_history::structured) failures: Vec<(PathBuf, String)>,
}

pub(in crate::usage_history::structured) fn repair_metadata(
    db: &mut Connection,
    home: &Path,
    cursor: Option<&MetadataCursor>,
) -> anyhow::Result<MetadataRepair> {
    let pending = {
        let mut query = db.prepare(
            "SELECT e.source, e.start, e.session, e.call_id, e.timestamp_ms, s.identity,
                 e.is_output, e.call_kind, e.name
             FROM execution_observations e JOIN usage_sources s ON s.source = e.source
             WHERE (e.source, e.start) > (?1, ?2)
             ORDER BY e.source, e.start LIMIT 64",
        )?;
        query.query_map(params![cursor.map_or(&[][..], |c| c.source.as_slice()),
            cursor.map_or(-1, |c| c.start)], |row| {
            Ok((row.get::<_, Vec<u8>>(0)?, row.get::<_, i64>(1)?, row.get::<_, String>(2)?,
                row.get::<_, String>(3)?, row.get::<_, i64>(4)?, row.get::<_, Vec<u8>>(5)?,
                row.get::<_, bool>(6)?, row.get::<_, Option<String>>(7)?,
                row.get::<_, Option<String>>(8)?))
        })?.collect::<Result<Vec<_>, _>>()?
    };
    let mut next = cursor.cloned();
    let mut failures = Vec::new();
    let mut recovered = Vec::new();
    let mut bytes_left = 4 * 1024 * 1024;
    let mut processed = 0;
    for (source, start, session, id, timestamp, identity, is_output, kind, name) in &pending {
        if bytes_left == 0 { break; }
        // 按已有主键分页后才筛选，分类已知的记录也推进游标，不额外占用索引页。
        next = Some(MetadataCursor { source: source.clone(), start: *start });
        processed += 1;
        let legacy_skill = kind.as_deref() == Some("skill") && matches!(name.as_deref(),
            Some("functions.exec_command" | "exec_command" | "functions.read_file" | "read_file"
                | "functions.shell" | "shell" | "functions.exec" | "exec" | "functions.read"
                | "read" | "functions.open_file" | "open_file"));
        if *is_output || kind.is_some() && !legacy_skill { continue; }
        let path = match decode_source(source) {
            Ok(path) => path,
            Err(error) => {
                failures.push((home.to_path_buf(),
                    format!("invalid execution metadata source in index: {error:#}")));
                continue;
            }
        };
        let result = (|| -> anyhow::Result<_> {
            ensure!((path.starts_with(home.join("sessions"))
                || path.starts_with(home.join("archived_sessions")))
                && fs::canonicalize(&path)? == path, "metadata source is outside session history");
            ensure!(path.extension().is_some_and(|ext| ext == "jsonl"),
                "compressed metadata source requires separate recovery");
            let mut file = File::open(&path)?;
            ensure!(encode_identity(codex_plus_core::codex_sqlite::opened_file_identity(&file)?)
                == *identity, "metadata source identity changed");
            file.seek(SeekFrom::Start(u64::try_from(*start)?))?;
            let mut bytes = Vec::new();
            BufReader::new(file.take(bytes_left as u64)).read_until(b'\n', &mut bytes)?;
            bytes_left -= bytes.len();
            ensure!(bytes.last() == Some(&b'\n'), "metadata line is incomplete or exceeds read budget");
            let item = parse_line(&bytes, *start as u64, *start as u64 + bytes.len() as u64,
                session, false)?.context("metadata position is not a tool call")?;
            ensure!(item.id == *id && item.timestamp_ms == *timestamp,
                "metadata call identity changed");
            let ObservationKind::Call { name, namespace, kind, .. } = item.kind else {
                anyhow::bail!("metadata position is not a tool call");
            };
            Ok((name, namespace, kind))
        })();
        match result {
            Ok(fields) => recovered.push((source, start, fields)),
            Err(error) => failures.push((path, format!("{error:#}"))),
        }
    }
    if !recovered.is_empty() {
        let tx = db.transaction()?;
        for (source, start, (name, namespace, kind)) in recovered {
            tx.execute("UPDATE execution_observations SET name = ?3, namespace = ?4, call_kind = ?5
                WHERE source = ?1 AND start = ?2 AND is_output = 0",
                params![source, start, name, namespace, kind])?;
        }
        tx.execute("UPDATE usage_meta SET revision = revision + 1 WHERE singleton = 1", [])?;
        tx.commit()?;
    }
    if processed == pending.len() && pending.len() < 64 { next = None; }
    Ok(MetadataRepair { next, failures })
}
