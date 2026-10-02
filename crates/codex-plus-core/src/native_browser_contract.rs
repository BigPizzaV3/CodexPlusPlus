use super::*;
use std::process::{Command, Stdio};

const INSPECTOR: &[u8] = include_bytes!("../../../assets/native-browser/inspect-service.cjs");

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct Binding {
    pub start: usize,
    pub end: usize,
    pub policy: String,
    pub metadata: String,
    pub contract_sha: String,
}

impl Binding {
    pub fn replace(&self, source: &[u8], control: &Path) -> Result<Vec<u8>> {
        fn identifier(value: &str) -> bool {
            !value.is_empty()
                && value.len() <= 128
                && value.bytes().enumerate().all(|(index, byte)| {
                    byte.is_ascii_alphabetic()
                        || matches!(byte, b'_' | b'$')
                        || (index > 0 && byte.is_ascii_digit())
                })
        }
        ensure!(
            self.contract_sha.len() == 64
                && self
                    .contract_sha
                    .bytes()
                    .all(|byte| byte.is_ascii_hexdigit())
                && identifier(&self.policy)
                && identifier(&self.metadata)
                && self.policy != self.metadata,
            "Invalid structural recovery contract"
        );
        let text = std::str::from_utf8(source)?;
        ensure!(
            self.start < self.end
                && text.get(self.start..self.end) == Some(self.policy.as_str())
                && !text.contains("cppNativeIdentificationReader"),
            "Structural callback position changed"
        );
        let control =
            serde_json::to_string(&control.to_str().context("Non-Unicode control path")?)?;
        let callback = format!(
            "cppNativeIdentificationReader(this.runtime,{},{},{control})",
            self.policy, self.metadata
        );
        let candidate_len = source
            .len()
            .checked_sub(self.end - self.start)
            .and_then(|length| length.checked_add(callback.len()))
            .and_then(|length| length.checked_add(1 + HELPER.len()))
            .context("Adapted service size overflow")?;
        ensure!(
            candidate_len as u64 <= MAX_SERVICE,
            "Adapted service exceeds the recovery size limit"
        );
        Ok(format!(
            "{}{callback}{}\n{HELPER}",
            &text[..self.start],
            &text[self.end..]
        )
        .into_bytes())
    }
}

fn pe_x64(bytes: &[u8]) -> bool {
    if !bytes.starts_with(b"MZ") || bytes.len() < 64 {
        return false;
    }
    let offset = u32::from_le_bytes(bytes[60..64].try_into().unwrap()) as usize;
    bytes.get(offset..offset.saturating_add(6)) == Some(b"PE\0\0\x64\x86".as_slice())
}

pub(super) fn inspect(node: &Path, state: &Path, source: &[u8], entry: &[u8]) -> Result<Binding> {
    ensure!(
        source.len() as u64 <= MAX_SERVICE,
        "Oversized service input"
    );
    let helper = state.join(format!("inspector-{}.cjs", sha(INSPECTOR)));
    let _guards = pin_parents(&helper)?;
    if helper.exists() {
        ensure!(
            read_regular(&helper, 4 * 1024 * 1024)? == INSPECTOR,
            "Inspector changed externally"
        );
    } else {
        write_new(&helper, INSPECTOR)?;
    }
    let request = serde_json::to_vec(&json!({
        "service": std::str::from_utf8(source)?,
        "entry": std::str::from_utf8(entry)?,
    }))?;
    let input_path = state.join(format!(
        ".inspector-input-{}-{}.json",
        std::process::id(),
        SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos(),
    ));
    write_new(&input_path, &request)?;
    let result = (|| {
        let input = File::open(&input_path)?;
        let mut command = Command::new(node);
        command
            .args(["--no-addons", "--max-old-space-size=256"])
            .arg(&helper)
            .arg("--stdin")
            .current_dir(state)
            .env_clear()
            .stdin(Stdio::from(input))
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        if let Some(system_root) = std::env::var_os("SystemRoot") {
            command.env("SystemRoot", system_root);
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x08000000);
        }
        let mut child = command
            .spawn()
            .context("Could not start native contract inspector")?;
        run_child(&mut child, Duration::from_secs(20))
    })();
    fs::remove_file(&input_path).context("Could not remove temporary inspector input")?;
    result
}

fn run_child(child: &mut std::process::Child, timeout: Duration) -> Result<Binding> {
    let started = std::time::Instant::now();
    let result = (|| -> Result<Binding> {
        let status = loop {
            if let Some(status) = child.try_wait()? {
                break status;
            }
            ensure!(
                started.elapsed() < timeout,
                "Native contract inspection timed out"
            );
            std::thread::sleep(Duration::from_millis(25));
        };
        ensure!(
            status.success(),
            "Browser service does not match the supported semantic contract"
        );
        let mut output = Vec::new();
        child
            .stdout
            .take()
            .context("Missing inspector output")?
            .take(4097)
            .read_to_end(&mut output)?;
        ensure!(output.len() <= 4096, "Oversized inspector result");
        Ok(serde_json::from_slice(&output)?)
    })();
    if result.is_err() {
        let _ = child.kill();
    }
    let _ = child.wait();
    result
}

pub(super) fn detect(paths: &BrowserPaths, key: &str) -> Result<RuntimeContract> {
    detect_with(paths, key, inspect)
}

fn detect_with(
    paths: &BrowserPaths,
    key: &str,
    inspector: impl FnOnce(&Path, &Path, &[u8], &[u8]) -> Result<Binding>,
) -> Result<RuntimeContract> {
    ensure!(key_valid(key), "Invalid runtime key");
    let runtime = paths.runtime_root.join(key);
    let _guards = pin_parents(&runtime.join(SERVICE))?;
    let manifest = read_regular(&runtime.join("manifest.json"), 1024 * 1024)?;
    let info: Value = serde_json::from_slice(&manifest).context("Invalid runtime manifest.json")?;
    let mut contract = RuntimeContract::for_manifest(&manifest, &runtime, String::new())?;
    ensure!(contract.adaptive, "Expected an adaptive runtime");
    ensure!(
        info["platform"] == "windows"
            && info["arch"] == "x64"
            && info["target"] == "windows-x64"
            && info["node_path"] == "bin/node.exe"
            && info["node_repl_path"] == "bin/node_repl.exe"
            && info["node_modules"] == "bin/node_modules",
        "Unsupported native runtime layout"
    );
    let package = read_regular(
        &runtime.join("bin/node_modules/@oai/browser-desktop/package.json"),
        1024 * 1024,
    )?;
    let package_info: Value = serde_json::from_slice(&package)?;
    ensure!(
        package_info["name"] == "@oai/browser-desktop"
            && package_info["type"] == "module"
            && package_info["exports"]["./service"] == "./scripts/browser-service.mjs",
        "Unsupported browser service export"
    );
    let mut files = vec![
        ("manifest.json", sha(&manifest)),
        (
            "bin/node_modules/@oai/browser-desktop/package.json",
            sha(&package),
        ),
    ];
    for file in ["bin/node.exe", "bin/node_repl.exe"] {
        let bytes = read_regular(&runtime.join(file), 128 * 1024 * 1024)?;
        ensure!(
            pe_x64(&bytes),
            "Unsupported native executable format: {file}"
        );
        files.push((file, sha(&bytes)));
    }
    let entry = read_regular(&runtime.join(CUA_ENTRY), 1024 * 1024)?;
    files.push((CUA_ENTRY, sha(&entry)));
    let dir = paths.state_root.join(key);
    let source = if dir.join("journal.json").exists() {
        recovery_material(paths, key)?.1
    } else {
        read_regular(&runtime.join(SERVICE), MAX_SERVICE)?
    };
    let binding = inspector(
        &runtime.join("bin/node.exe"),
        &paths.state_root,
        &source,
        &entry,
    )?;
    // Validate byte offsets, identifiers and replacement without touching the service.
    binding.replace(&source, &paths.state_root.join("control.json"))?;
    contract.service_sha = sha(&source);
    contract.files = files
        .into_iter()
        .map(|(file, hash)| (file, FileCheck::Known(hash)))
        .collect();
    contract.binding = Some(binding);
    Ok(contract)
}

#[cfg(test)]
pub(super) fn synthetic_runtime(temp: &tempfile::TempDir) -> (BrowserPaths, &'static str) {
    let paths = BrowserPaths {
        codex_home: temp.path().join("home"),
        runtime_root: temp.path().join("runtimes"),
        state_root: temp.path().join("state"),
    };
    let key = "0123456789abcdef";
    let runtime = paths.runtime_root.join(key);
    let mut pe = vec![0; 128];
    pe[..2].copy_from_slice(b"MZ");
    pe[60..64].copy_from_slice(&64u32.to_le_bytes());
    pe[64..70].copy_from_slice(b"PE\0\0\x64\x86");
    let manifest = serde_json::to_vec(&json!({
        "name": "@oai/cua-node", "version": "0.1.0",
        "platform": "windows", "arch": "x64", "target": "windows-x64",
        "node_path": "bin/node.exe", "node_repl_path": "bin/node_repl.exe",
        "node_modules": "bin/node_modules",
    }))
    .unwrap();
    for (file, bytes) in [
        (MANIFEST, manifest.as_slice()),
        ("bin/node.exe", pe.as_slice()),
        ("bin/node_repl.exe", pe.as_slice()),
        (CUA_ENTRY, b"import * as cua from '@oai/cua-repl'; await cua.launch();".as_slice()),
        ("bin/node_modules/@oai/browser-desktop/package.json",
            br#"{"name":"@oai/browser-desktop","type":"module","exports":{"./service":"./scripts/browser-service.mjs"}}"#.as_slice()),
        (SERVICE, include_bytes!("../../../assets/native-browser/fixtures/service-specimen.mjs").as_slice()),
    ] {
        let path = runtime.join(file);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, bytes).unwrap();
    }
    fs::create_dir_all(&paths.state_root).unwrap();
    (paths, key)
}

#[cfg(test)]
pub(super) fn detect_synthetic(paths: &BrowserPaths, key: &str) -> Result<RuntimeContract> {
    // CI supplies Node 22. Fake PE fixtures are never executed.
    let executable = if cfg!(windows) { "node.exe" } else { "node" };
    let node =
        std::env::split_paths(&std::env::var_os("PATH").context("Node test PATH is missing")?)
            .map(|dir| dir.join(executable))
            .find(|path| path.is_file())
            .context("Synthetic browser tests require Node on PATH")?
            .canonicalize()?;
    detect_with(paths, key, |_, state, service, entry| {
        inspect(&node, state, service, entry)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recovery_binding_is_bounded_and_rejects_injected_identifiers() {
        let source =
            b"new cls(r,this.clientApi,()=>meta(this.runtime),this.turnEndedTracker,policy)";
        let start = source.len() - 7;
        let binding = Binding {
            start,
            end: start + 6,
            policy: "policy".into(),
            metadata: "meta".into(),
            contract_sha: "a".repeat(64),
        };
        assert!(
            binding
                .replace(source, Path::new("C:/state/control.json"))
                .is_ok()
        );
        for bad in ["meta);evil(", "", "1name", "name.path"] {
            let mut changed = binding.clone();
            changed.metadata = bad.into();
            assert!(
                changed
                    .replace(source, Path::new("C:/state/control.json"))
                    .is_err()
            );
        }
        let mut changed = binding.clone();
        changed.end = usize::MAX;
        assert!(
            changed
                .replace(source, Path::new("C:/state/control.json"))
                .is_err()
        );
        changed = binding;
        changed.contract_sha = "unknown".into();
        assert!(
            changed
                .replace(source, Path::new("C:/state/control.json"))
                .is_err()
        );
    }

    #[test]
    fn executable_format_is_not_inferred_from_the_filename() {
        assert!(!pe_x64(b"fixture-node.exe"));
        let mut bytes = vec![0; 128];
        bytes[..2].copy_from_slice(b"MZ");
        bytes[60..64].copy_from_slice(&64u32.to_le_bytes());
        bytes[64..70].copy_from_slice(b"PE\0\0\x64\x86");
        assert!(pe_x64(&bytes));
        bytes[68] = 0x4c;
        assert!(!pe_x64(&bytes));
    }

    #[test]
    fn adapted_service_must_fit_the_same_limit_used_by_recovery() {
        let mut source = vec![b' '; MAX_SERVICE as usize];
        let start = source.len() - 6;
        source[start..].copy_from_slice(b"policy");
        let binding = Binding {
            start,
            end: source.len(),
            policy: "policy".into(),
            metadata: "meta".into(),
            contract_sha: "b".repeat(64),
        };
        let error = binding
            .replace(&source, Path::new("C:/state/control.json"))
            .unwrap_err();
        assert!(error.to_string().contains("recovery size limit"));
    }

    #[test]
    fn synthetic_detect_transactions_guards_and_no_node_recovery() {
        let temp = tempfile::tempdir().unwrap();
        let (paths, key) = synthetic_runtime(&temp);
        let service = paths.runtime_root.join(key).join(SERVICE);
        let original = fs::read(&service).unwrap();
        let modified = fs::metadata(&service).unwrap().modified().unwrap();
        let contract = detect_synthetic(&paths, key).unwrap();
        assert!(contract.adaptive && contract.binding.is_some());
        assert!(
            contract
                .files
                .iter()
                .all(|(_, check)| matches!(check, FileCheck::Known(_)))
        );
        prepare(&paths, key, &contract).unwrap();
        let candidate = fs::read(&service).unwrap();
        prepare(&paths, key, &contract).unwrap();
        assert_eq!(fs::read(&service).unwrap(), candidate);
        let again = detect_synthetic(&paths, key).unwrap();
        assert_eq!(again.service_sha, sha(&original));
        assert_eq!(
            again.binding.as_ref().unwrap().contract_sha,
            contract.binding.as_ref().unwrap().contract_sha
        );
        let worker = paths.runtime_root.join(key).join("bin/node_repl.exe");
        fs::write(&worker, b"changed worker").unwrap();
        assert!(prepare(&paths, key, &again).is_err());
        fs::remove_file(paths.runtime_root.join(key).join("bin/node.exe")).unwrap();
        restore_all(&paths, None).unwrap();
        assert_eq!(fs::read(&service).unwrap(), original);
        assert_eq!(
            fs::metadata(&service).unwrap().modified().unwrap(),
            modified
        );
        assert!(!fs::read_dir(&paths.state_root).unwrap().any(|entry| {
            entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".inspector-input")
        }));
    }

    #[test]
    fn synthetic_detect_refuses_version_layout_export_and_executable_drift() {
        for (file, bytes) in [
            (MANIFEST, br#"{"name":"x","version":"0.0.10"}"#.as_slice()),
            (MANIFEST, br#"{"name":"x","version":"latest"}"#.as_slice()),
            (
                MANIFEST,
                br#"{"name":"x","version":"0.1.0","platform":"linux"}"#.as_slice(),
            ),
            ("bin/node.exe", b"not a PE".as_slice()),
            (
                "bin/node_modules/@oai/browser-desktop/package.json",
                br#"{"name":"other","type":"module"}"#.as_slice(),
            ),
            (CUA_ENTRY, b"await other.launch();".as_slice()),
            (SERVICE, b"const nope = 1;".as_slice()),
        ] {
            let temp = tempfile::tempdir().unwrap();
            let (paths, key) = synthetic_runtime(&temp);
            fs::write(paths.runtime_root.join(key).join(file), bytes).unwrap();
            assert!(detect_synthetic(&paths, key).is_err(), "{file}");
            assert!(!paths.state_root.join(key).join("journal.json").exists());
        }
    }

    #[cfg(windows)]
    #[test]
    fn deadline_reaps_a_child_that_never_returns() {
        use std::os::windows::process::CommandExt;
        let shell = PathBuf::from(std::env::var_os("SystemRoot").unwrap())
            .join("System32/WindowsPowerShell/v1.0/powershell.exe");
        let mut child = Command::new(shell)
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "Start-Sleep -Seconds 60",
            ])
            .creation_flags(0x08000000)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let started = std::time::Instant::now();
        let error = run_child(&mut child, Duration::from_millis(150)).unwrap_err();
        assert!(error.to_string().contains("timed out"), "{error}");
        assert!(started.elapsed() < Duration::from_secs(5));
        assert!(child.try_wait().unwrap().is_some());
    }
}
