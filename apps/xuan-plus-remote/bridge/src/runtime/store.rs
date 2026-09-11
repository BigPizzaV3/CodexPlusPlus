use std::collections::BTreeSet;
use std::path::Path;

use anyhow::{Context, bail};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use ring::signature::{Ed25519KeyPair, KeyPair};
use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Serialize, Deserialize)]
#[serde(default)]
pub(super) struct SavedState {
    pub enabled: bool,
    pub enrolled: bool,
    pub pc_id: String,
    pub installation_id: String,
    pub epoch: u64,
    pub version: u64,
    pub auto_sync: bool,
    pub selected: BTreeSet<String>,
    pub removed: BTreeSet<String>,
}

impl Default for SavedState {
    fn default() -> Self {
        Self {
            enabled: false,
            enrolled: false,
            pc_id: String::new(),
            installation_id: String::new(),
            epoch: 0,
            version: 0,
            auto_sync: true,
            selected: BTreeSet::new(),
            removed: BTreeSet::new(),
        }
    }
}

pub(super) struct Store {
    db: Connection,
    pub key: Ed25519KeyPair,
    pub state: SavedState,
}

pub(super) struct CommandReceipt {
    pub payload_digest: String,
    pub status: String,
    pub error_code: Option<String>,
}

impl Store {
    pub fn open(path: &Path) -> anyhow::Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut db = Connection::open(path)?;
        db.busy_timeout(std::time::Duration::from_secs(3))?;
        db.execute_batch(
            "PRAGMA secure_delete=ON;
             CREATE TABLE IF NOT EXISTS mobile_identity (
               singleton INTEGER PRIMARY KEY CHECK(singleton=1),
               protected_key BLOB NOT NULL, state TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS mobile_command_receipts (
               command_id TEXT PRIMARY KEY,
               payload_digest TEXT NOT NULL,
               status TEXT NOT NULL,
               error_code TEXT,
               created_at INTEGER NOT NULL
             );",
        )?;
        let saved: Option<(Vec<u8>, String)> = db
            .query_row(
                "SELECT protected_key, state FROM mobile_identity WHERE singleton=1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        let (key, state) = if let Some((encrypted, state)) = saved {
            let mut plain = protect(&encrypted, false)?;
            let result = Ed25519KeyPair::from_pkcs8(&plain);
            plain.fill(0);
            (
                result.map_err(|_| anyhow::anyhow!("无法读取本机设备身份"))?,
                serde_json::from_str(&state).context("无法读取手机连接状态")?,
            )
        } else {
            let document = Ed25519KeyPair::generate_pkcs8(&ring::rand::SystemRandom::new())
                .map_err(|_| anyhow::anyhow!("无法生成本机设备身份"))?;
            let encrypted = protect(document.as_ref(), true)?;
            let key = Ed25519KeyPair::from_pkcs8(document.as_ref())
                .map_err(|_| anyhow::anyhow!("无法生成本机设备身份"))?;
            let state = SavedState {
                pc_id: super::id(),
                installation_id: super::id(),
                epoch: 1,
                ..Default::default()
            };
            let tx = db.transaction()?;
            tx.execute(
                "INSERT INTO mobile_identity VALUES (1, ?1, ?2)",
                rusqlite::params![encrypted, serde_json::to_string(&state)?],
            )?;
            tx.commit()?;
            (key, state)
        };
        Ok(Self { db, key, state })
    }

    pub fn save(&self) -> anyhow::Result<()> {
        self.db.execute(
            "UPDATE mobile_identity SET state=?1 WHERE singleton=1",
            [serde_json::to_string(&self.state)?],
        )?;
        Ok(())
    }

    pub fn version(&mut self) -> anyhow::Result<u64> {
        self.state.version = self
            .state
            .version
            .checked_add(1)
            .filter(|v| *v <= 9_007_199_254_740_991)
            .context("同步版本已超出允许范围")?;
        // 先落盘再发送；重连重建快照也不能回退到云端已接收的版本。
        self.save()?;
        Ok(self.state.version)
    }

    pub fn headers(&self, method: &str, path: &str, body: &[u8]) -> Vec<(&'static str, String)> {
        let timestamp = super::now();
        let nonce = super::id();
        let canonical = format!(
            "workagents-pc-request-v1\n{method}\n{path}\ndev\n{}\n{}\n{timestamp}\n{nonce}\n{:x}",
            self.state.pc_id,
            self.state.installation_id,
            Sha256::digest(body),
        );
        let mut public_der = vec![
            0x30, 0x2a, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x03, 0x21, 0x00,
        ];
        public_der.extend_from_slice(self.key.public_key().as_ref());
        vec![
            ("x-workagents-pc-device-id", self.state.pc_id.clone()),
            (
                "x-workagents-pc-installation-id",
                self.state.installation_id.clone(),
            ),
            ("x-workagents-pc-timestamp", timestamp),
            ("x-workagents-pc-nonce", nonce),
            (
                "x-workagents-pc-public-key",
                URL_SAFE_NO_PAD.encode(public_der),
            ),
            (
                "x-workagents-pc-signature",
                URL_SAFE_NO_PAD.encode(self.key.sign(canonical.as_bytes()).as_ref()),
            ),
        ]
    }

    pub fn command_receipt(&self, command_id: &str) -> anyhow::Result<Option<CommandReceipt>> {
        Ok(self
            .db
            .query_row(
                "SELECT payload_digest, status, error_code FROM mobile_command_receipts
                 WHERE command_id=?1",
                [command_id],
                |row| {
                    Ok(CommandReceipt {
                        payload_digest: row.get(0)?,
                        status: row.get(1)?,
                        error_code: row.get(2)?,
                    })
                },
            )
            .optional()?)
    }

    pub fn save_command_receipt(
        &mut self,
        command_id: &str,
        payload_digest: &str,
        status: &str,
        error_code: Option<&str>,
    ) -> anyhow::Result<()> {
        let tx = self.db.transaction()?;
        tx.execute(
            "INSERT OR IGNORE INTO mobile_command_receipts
             (command_id, payload_digest, status, error_code, created_at)
             VALUES (?1, ?2, ?3, ?4, unixepoch())",
            rusqlite::params![command_id, payload_digest, status, error_code],
        )?;
        tx.execute(
            "DELETE FROM mobile_command_receipts WHERE command_id IN (
               SELECT command_id FROM mobile_command_receipts
               ORDER BY created_at DESC, rowid DESC LIMIT -1 OFFSET 256
             )",
            [],
        )?;
        tx.commit()?;
        Ok(())
    }
}

#[cfg(windows)]
fn protect(bytes: &[u8], encrypt: bool) -> anyhow::Result<Vec<u8>> {
    use windows::Win32::Foundation::{HLOCAL, LocalFree};
    use windows::Win32::Security::Cryptography::{
        CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN, CryptProtectData, CryptUnprotectData,
    };
    let input = CRYPT_INTEGER_BLOB {
        cbData: bytes.len().try_into()?,
        pbData: bytes.as_ptr().cast_mut(),
    };
    let mut output = CRYPT_INTEGER_BLOB::default();
    // DPAPI 绑定当前 Windows 用户，数据库中不保存明文私钥。
    unsafe {
        let result = if encrypt {
            CryptProtectData(
                &input,
                windows::core::PCWSTR::null(),
                None,
                None,
                None,
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        } else {
            CryptUnprotectData(
                &input,
                None,
                None,
                None,
                None,
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        };
        if result.is_err() {
            bail!("系统无法保护或解锁本机设备身份");
        }
        let result = std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec();
        std::slice::from_raw_parts_mut(output.pbData, output.cbData as usize).fill(0);
        let _ = LocalFree(HLOCAL(output.pbData.cast()));
        Ok(result)
    }
}

#[cfg(not(windows))]
fn protect(_bytes: &[u8], _encrypt: bool) -> anyhow::Result<Vec<u8>> {
    bail!("手机连接的设备身份保护目前仅支持 Windows")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(windows)]
    fn identity_and_monotonic_versions_survive_restart_without_plaintext_key() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("remote.sqlite");
        let mut store = Store::open(&path).unwrap();
        let identity = store.state.pc_id.clone();
        let public = store.key.public_key().as_ref().to_vec();
        assert_eq!(store.version().unwrap(), 1);
        store.state.selected.insert("test_task_00000001".into());
        store.save().unwrap();
        drop(store);
        let mut store = Store::open(&path).unwrap();
        assert_eq!(identity, store.state.pc_id);
        assert_eq!(public, store.key.public_key().as_ref());
        assert_eq!(store.version().unwrap(), 2);
        assert!(store.state.auto_sync);
        assert!(store.state.selected.contains("test_task_00000001"));
        let encrypted: Vec<u8> = store
            .db
            .query_row("SELECT protected_key FROM mobile_identity", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert!(Ed25519KeyPair::from_pkcs8(&encrypted).is_err());
    }

    #[test]
    fn legacy_state_enables_automatic_sync_after_upgrade() {
        let state: SavedState =
            serde_json::from_str(r#"{"enabled":true,"selected":["test_task_00000001"]}"#).unwrap();
        assert!(state.auto_sync);
        assert!(state.selected.contains("test_task_00000001"));
    }

    #[test]
    #[cfg(windows)]
    fn command_receipts_survive_restart_without_storing_payload_text() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("remote.sqlite");
        let mut store = Store::open(&path).unwrap();
        store
            .save_command_receipt("command_receipt_0001", &"a".repeat(64), "completed", None)
            .unwrap();
        drop(store);
        let store = Store::open(&path).unwrap();
        let receipt = store
            .command_receipt("command_receipt_0001")
            .unwrap()
            .unwrap();
        assert_eq!(receipt.payload_digest, "a".repeat(64));
        assert_eq!(receipt.status, "completed");
        assert!(receipt.error_code.is_none());
        let schema: String = store
            .db
            .query_row(
                "SELECT sql FROM sqlite_master WHERE name='mobile_command_receipts'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(!schema.contains("payload_text"));
    }
}
