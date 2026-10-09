//! Opt-in adaptation of a verified native Edge/Chrome identification callback.
//! Does not implement browser execution, cloud identity or approval decisions.

use std::collections::BTreeSet;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, ensure};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

#[path = "native_browser_contract.rs"]
mod structural;

const SERVICE: &str = "bin/node_modules/@oai/browser-desktop/scripts/browser-service.mjs";
const ORIGINAL_SHA: &str = "3e6fd4a8cf09f57549d63f2c9cbfa2abf42f0a6b0c09c3d6605fe07c8ba09e4a";
const NATIVE_SHA: &str = "ef53f8f0d957b7cf437020499b6b9d880dee381214788930107b549237f7949c";
const CURRENT_ORIGINAL_SHA: &str =
    "fc0660ba45e6c10b532d8faa0c1bac704d987dad3d4b74478f49fdd82bf90086";
const CURRENT_MANIFEST_SHA: &str =
    "2c8ea57bfab596fb3b9cf78673b62a763f8d484aa8d380e341324354ce9e90e8";
const HELPER: &str = include_str!("../../../assets/native-browser/require-identification.mjs");
const MAX_SERVICE: u64 = 32 * 1024 * 1024;

/// issue #2294 / #2209：结构不变量取代「再堆第三个哈希常量」。
/// 0.0.11 → 0.0.24 已经证明「一个版本发一次就作废整张表」，所以未知哈希不再直接失败，
/// 而是退回结构校验；只有结构不变量本身被破坏时才拒绝。
const CUA_ENTRY: &str = "bin/node_modules/@oai/cua-repl/bin/cua-repl.mjs";
const MANIFEST: &str = "manifest.json";
const MIN_RUNTIME_VERSION: (u32, u32, u32) = (0, 0, 11);

#[derive(Debug)]
struct UnverifiedRuntime(String);

impl std::fmt::Display for UnverifiedRuntime {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "Legacy compatibility is not verified for this runtime: {}", self.0)
    }
}

impl std::error::Error for UnverifiedRuntime {}

/// 每个必需组件要么给出「已知良好」的哈希，要么只校验结构。
/// `Structural` 是 issue #2294 的降级通道：未知哈希不再让整条链路 fail-closed。
#[derive(Clone)]
enum FileCheck {
    Known(String),
    Structural,
}

#[derive(Clone)]
struct RuntimeContract {
    service_sha: String,
    files: Vec<(&'static str, FileCheck)>,
    /// 未知版本经结构校验后接受时置位，用于状态展示与诊断留痕。
    adaptive: bool,
    binding: Option<structural::Binding>,
}

/// 新旧两代的共享组件清单：浏览器服务始终单独校验，其余在此登记。
const RUNTIME_FILES: [&str; 4] = ["bin/node_repl.exe", "bin/node.exe", MANIFEST, CUA_ENTRY];

impl RuntimeContract {
    fn pinned() -> Self {
        Self {
            service_sha: ORIGINAL_SHA.into(),
            binding: None,
            files: vec![
                ("bin/node_repl.exe", FileCheck::Known(NATIVE_SHA.into())),
                (
                    "bin/node.exe",
                    FileCheck::Known(
                        "be14417b6c4b4a5af06be7c16bda58730f26b912c3e8c6489d12392ef08f35bf".into(),
                    ),
                ),
                (
                    MANIFEST,
                    FileCheck::Known(
                        "ba3691b0717b6df8064c3841a75c784e8af9633c7b47f2fdb56d8de099efe6fc".into(),
                    ),
                ),
                (
                    CUA_ENTRY,
                    FileCheck::Known(
                        "992174a5e637645aeb444adfdb1bae688e997bb84d7db07532f68e358e60f278".into(),
                    ),
                ),
            ],
            adaptive: false,
        }
    }

    fn current() -> Self {
        Self {
            service_sha: CURRENT_ORIGINAL_SHA.into(),
            binding: None,
            files: vec![
                (
                    "bin/node_repl.exe",
                    FileCheck::Known(
                        "e42e0d846b9c1e5da3ec7b5e069fdae3643df590f4e304f433cfaa7fbd8732a7".into(),
                    ),
                ),
                (
                    "bin/node.exe",
                    FileCheck::Known(
                        "d3c3c290b11d55ef747e63f5a63538e0d8ca95f3f9668bb6a8081a25ba2befab".into(),
                    ),
                ),
                (MANIFEST, FileCheck::Known(CURRENT_MANIFEST_SHA.into())),
                (
                    CUA_ENTRY,
                    FileCheck::Known(
                        "992174a5e637645aeb444adfdb1bae688e997bb84d7db07532f68e358e60f278".into(),
                    ),
                ),
            ],
            adaptive: false,
        }
    }

    /// 未知 manifest 版本的降级契约：组件只校验结构，服务本体的 SHA 由
    /// `resolve_contract` 现算，绝不写死。
    fn adapted(service_sha: String) -> Self {
        Self {
            service_sha,
            binding: None,
            files: RUNTIME_FILES
                .iter()
                .map(|file| (*file, FileCheck::Structural))
                .collect(),
            adaptive: true,
        }
    }

    /// 已知良好版本走哈希快路径；未知版本走结构校验（issue #2294）。
    /// `manifest` 是 manifest.json 的原始字节，`runtime` 是其所在目录，
    /// 结构校验需要落到文件系统确认必需组件确实存在。
    fn for_manifest(manifest: &[u8], runtime: &Path, service_sha: String) -> Result<Self> {
        match sha(manifest).as_str() {
            "ba3691b0717b6df8064c3841a75c784e8af9633c7b47f2fdb56d8de099efe6fc" => {
                Ok(Self::pinned())
            }
            CURRENT_MANIFEST_SHA => Ok(Self::current()),
            _ => {
                let parsed = parse_manifest(manifest)?;
                ensure!(
                    parsed.at_least(MIN_RUNTIME_VERSION),
                    "Native runtime is older than the supported minimum"
                );
                for file in RUNTIME_FILES {
                    let path = runtime.join(file);
                    ensure!(path.exists(), "Native runtime is missing {file}");
                    let meta = fs::metadata(&path)?;
                    ensure!(
                        meta.is_file() && meta.len() <= 128 * 1024 * 1024,
                        "Unexpected native runtime component: {file}"
                    );
                }
                Ok(Self::adapted(service_sha))
            }
        }
    }
}

struct ManifestVersion {
    parts: (u32, u32, u32),
    raw: String,
}

impl ManifestVersion {
    fn at_least(&self, minimum: (u32, u32, u32)) -> bool {
        self.parts >= minimum
    }
}

impl std::fmt::Debug for ManifestVersion {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "ManifestVersion({})", self.raw)
    }
}

/// 解析 manifest.json 的结构不变量：必要字段、版本号可解析且不低于下限。
/// 语言/时区差异、额外字段、字段顺序都不得影响判定。
fn parse_manifest(manifest: &[u8]) -> Result<ManifestVersion> {
    let value: Value = serde_json::from_slice(manifest)?;
    let object = value.as_object().context("Native runtime manifest is not an object")?;
    // Desktop's generated manifests use archive identity, not package name/version.
    if !object.contains_key("name") && !object.contains_key("version") {
        let archive = value["runtime_archive_version"]
            .as_str()
            .context("Native runtime manifest is missing runtime_archive_version")?;
        let (version, build) = archive
            .split_once('/')
            .context("Invalid runtime archive version")?;
        ensure!(
            !build.is_empty() && build.len() <= 128
                && build.bytes().all(|byte| byte.is_ascii_alphanumeric() || byte == b'-'),
            "Invalid runtime archive build"
        );
        ensure!(
            value["runtime_archive_name"].as_str()
                == Some(format!("cua-node-{version}-{build}-windows-x64.zip").as_str()),
            "Conflicting runtime archive identity"
        );
        return parse_version(version).context("Invalid runtime archive version");
    }
    for required in ["name", "version"] {
        ensure!(
            object.get(required).is_some_and(|field| !field.is_null()),
            "Native runtime manifest is missing {required}"
        );
    }
    let raw = value["version"]
        .as_str()
        .context("Native runtime version is not a string")?
        .trim()
        .to_owned();
    parse_version(&raw).with_context(|| format!("Unsupported native runtime version: {raw}"))
}

/// 只接受 `<major>.<minor>.<patch>` 形状（允许前后空白与 `v` 前缀）。
/// 预发布后缀不参与比较：契约只关心「不低于某个功能下限」。
fn parse_version(raw: &str) -> Result<ManifestVersion> {
    let core = raw.trim().trim_start_matches(['v', 'V']);
    let core = core.split(['-', '+']).next().unwrap_or_default();
    let segments: Vec<&str> = core.split('.').collect();
    let [major, minor, patch] = segments.as_slice() else {
        anyhow::bail!("Expected a major.minor.patch version");
    };
    let number = |part: &str, name: &str| -> Result<u32> {
        part.trim()
            .parse::<u32>()
            .with_context(|| format!("Invalid {name} version"))
    };
    Ok(ManifestVersion {
        parts: (
            number(major, "major")?,
            number(minor, "minor")?,
            number(patch, "patch")?,
        ),
        raw: raw.into(),
    })
}

#[derive(Debug, Clone)]
pub struct BrowserPaths {
    pub codex_home: PathBuf,
    pub runtime_root: PathBuf,
    pub state_root: PathBuf,
}

impl BrowserPaths {
    pub fn current() -> Result<Self> {
        ensure!(
            cfg!(windows),
            "Native browser compatibility is Windows-only"
        );
        let local = std::env::var_os("LOCALAPPDATA").context("LOCALAPPDATA is unavailable")?;
        Ok(Self {
            codex_home: crate::codex_home::default_codex_home_dir(),
            runtime_root: PathBuf::from(local).join("OpenAI/Codex/runtimes/cua_node"),
            state_root: crate::paths::default_app_state_dir().join("native-browser-identification"),
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BrowserStatus {
    pub state: String,
    pub detail: String,
}

impl BrowserStatus {
    fn new(state: &str, detail: &str) -> Self {
        Self {
            state: state.into(),
            detail: detail.into(),
        }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Journal {
    schema: u32,
    original_sha: String,
    candidate_sha: String,
    modified_secs: u64,
    modified_nanos: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    binding: Option<structural::Binding>,
}

fn sha(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn key_valid(key: &str) -> bool {
    key.len() == 16 && key.bytes().all(|b| b.is_ascii_hexdigit())
}

// Reject junctions as well as symlinks, including in parent directories.
fn plain_path(path: &Path) -> Result<()> {
    ensure!(path.is_absolute(), "Expected an absolute local path");
    for ancestor in path.ancestors() {
        if let Ok(meta) = fs::symlink_metadata(ancestor) {
            ensure!(
                !meta.file_type().is_symlink(),
                "Linked paths are unsupported"
            );
            #[cfg(windows)]
            {
                use std::os::windows::fs::MetadataExt;
                ensure!(
                    meta.file_attributes() & 0x400 == 0,
                    "Reparse paths are unsupported"
                );
            }
        }
    }
    ensure!(
        !path
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir)),
        "Parent traversal is unsupported"
    );
    Ok(())
}

// Deny directory deletion/renaming while a Windows transaction uses its descendants.
// Open root-first with OPEN_REPARSE_POINT so no checked parent can become a junction.
fn pin_parents(path: &Path) -> Result<Vec<File>> {
    plain_path(path)?;
    let mut guards = Vec::new();
    #[cfg(windows)]
    {
        use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
        let mut parents: Vec<_> = path.ancestors().skip(1).collect();
        parents.reverse();
        for parent in parents {
            if !parent.exists() {
                break;
            }
            let guard = OpenOptions::new()
                .read(true)
                .share_mode(0x1 | 0x2) // FILE_SHARE_READ | FILE_SHARE_WRITE, never DELETE.
                .custom_flags(0x02000000 | 0x00200000) // BACKUP_SEMANTICS | OPEN_REPARSE_POINT.
                .open(parent)?;
            let meta = guard.metadata()?;
            ensure!(
                meta.is_dir() && meta.file_attributes() & 0x400 == 0,
                "Parent directory is a reparse point"
            );
            guards.push(guard);
        }
    }
    Ok(guards)
}

fn read_regular(path: &Path, limit: u64) -> Result<Vec<u8>> {
    let _guards = pin_parents(path)?;
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(0x1).custom_flags(0x00200000);
    }
    let file = options.open(path)?;
    let meta = file.metadata()?;
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        ensure!(
            meta.file_attributes() & 0x400 == 0,
            "File is a reparse point"
        );
    }
    ensure!(
        meta.is_file() && meta.len() <= limit,
        "Unexpected file type or size"
    );
    let mut bytes = Vec::new();
    file.take(limit + 1).read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() as u64 <= limit,
        "File grew beyond the size limit"
    );
    Ok(bytes)
}

fn write_new(path: &Path, bytes: &[u8]) -> Result<File> {
    let _guards = pin_parents(path)?;
    let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(file)
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    atomic_write_with_modified(path, bytes, None)
}

fn atomic_write_with_modified(
    path: &Path,
    bytes: &[u8],
    modified: Option<SystemTime>,
) -> Result<()> {
    let _guards = pin_parents(path)?;
    let temp = path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| -> Result<()> {
        let file = write_new(&temp, bytes)?;
        // Publish bytes and timestamp together; never reopen the replaced target to set metadata.
        if let Some(modified) = modified {
            file.set_modified(modified)?;
            file.sync_all()?;
        }
        drop(file);
        #[cfg(windows)]
        {
            use std::os::windows::ffi::OsStrExt;
            use windows::Win32::Storage::FileSystem::{
                MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
            };
            use windows::core::PCWSTR;
            let source: Vec<u16> = temp.as_os_str().encode_wide().chain(Some(0)).collect();
            let target: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
            unsafe {
                MoveFileExW(
                    PCWSTR(source.as_ptr()),
                    PCWSTR(target.as_ptr()),
                    MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
                )
            }
            .map_err(anyhow::Error::from)
        }
        #[cfg(not(windows))]
        {
            fs::rename(&temp, path).map_err(anyhow::Error::from)
        }
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}

fn transform(source: &[u8], control: &Path, contract: &RuntimeContract) -> Result<Vec<u8>> {
    ensure!(
        sha(source) == contract.service_sha,
        UnverifiedRuntime("browser service".into())
    );
    transform_binding(source, control, contract)
}

/// 从一次回调绑定里摘出的四个压缩标识符。
/// 构造器名不必保留（替换串自带），但元数据回调与 policy 回调必须原样回填。
struct Binding {
    start: usize,
    end: usize,
    constructor: String,
    runtime_getter: String,
    policy: String,
}

impl Binding {
    fn original(&self) -> String {
        format!(
            "new {}(r,this.clientApi,()=>{}(this.runtime),this.turnEndedTracker,{})",
            self.constructor, self.runtime_getter, self.policy
        )
    }
}

/// 整段文本里符合绑定形状的位置数。压缩名可以各不相同，所以不能只比字面量：
/// 两处不同世代或不同名字的绑定同样算冲突（issue #2294）。
fn matching_bindings(text: &str) -> usize {
    let mut count = 0;
    let mut cursor = 0;
    while let Some(found) = text[cursor..].find("this.turnEndedTracker") {
        let live = cursor + found;
        cursor = live + "this.turnEndedTracker".len();
        if let Some(start) = text[..live].rfind("new ") {
            if find_binding(&text[start..]).is_some_and(|binding| binding.start == 0) {
                count += 1;
            }
        }
    }
    count
}

fn identifier_len(text: &str, start: usize) -> usize {
    let mut len = 0;
    for (offset, ch) in text[start..].char_indices() {
        if !(ch.is_ascii_alphanumeric() || ch == '_' || ch == '$') {
            return offset;
        }
        len = offset + ch.len_utf8();
    }
    len
}

/// 结构正则的等价手写实现（本 crate 无 regex 依赖）：
/// `new \w+\(r,this\.clientApi,\(\)=>\w+\(this\.runtime\),this\.turnEndedTracker,\w+\)`
/// 只认形状不认压缩名，所以 `nf/ze/cD` 与 `uh/We/wv` 都能吃下（issue #2294）。
fn find_binding(text: &str) -> Option<Binding> {
    /// 前进一个固定字面量，返回其后是否跟上指定字面量。
    fn expect(text: &str, cursor: &mut usize, literal: &str) -> Option<()> {
        let rest = text.get(*cursor..)?;
        rest.strip_prefix(literal)?;
        *cursor += literal.len();
        Some(())
    }

    /// 前进一个 JS 标识符并返回它；空串视为不匹配。
    fn identifier(text: &str, cursor: &mut usize) -> Option<String> {
        let len = identifier_len(text, *cursor);
        let name = text.get(*cursor..*cursor + len)?;
        (*cursor) += len;
        (!name.is_empty()).then(|| name.to_owned())
    }

    // 真实绑定形如 `new eh(r,this.clientApi,()=>je(this.runtime),this.turnEndedTracker,sv)`：
    // `new `、`r`、`this.clientApi`、`this.runtime`、`this.turnEndedTracker` 都是稳定形状，
    // 只有四个压缩标识符会随发版变化，所以逐个按结构吃下、再原样回填。
    let live = text.find("this.turnEndedTracker")?;
    let start = text[..live].rfind("new ")?;
    let mut cursor = start;
    expect(text, &mut cursor, "new ")?;
    let constructor = identifier(text, &mut cursor)?;
    expect(text, &mut cursor, "(r,this.clientApi,()=>")?;
    let runtime_getter = identifier(text, &mut cursor)?;
    // 元数据回调名已在上一步被吃下，这里接的是它的实参列表。
    expect(text, &mut cursor, "(this.runtime),this.turnEndedTracker,")?;
    let policy = identifier(text, &mut cursor)?;
    expect(text, &mut cursor, ")")?;

    Some(Binding {
        start,
        end: cursor,
        constructor,
        runtime_getter,
        policy,
    })
}

fn count_occurrences(text: &str, needle: &str) -> usize {
    text.match_indices(needle).count()
}

fn transform_binding(source: &[u8], control: &Path, contract: &RuntimeContract) -> Result<Vec<u8>> {
    if let Some(binding) = &contract.binding {
        return binding.replace(source, control);
    }
    let text = std::str::from_utf8(source)?;
    ensure!(
        !text.contains("cppNativeIdentificationReader"),
        "Conflicting adapter"
    );
    let binding = find_binding(text).context("Expected one callback binding")?;
    let original = binding.original();
    // 唯一命中：整段文本里这种形状的回调绑定只能有一处（压缩名可以各不相同），
    // 第二处一律拒绝——猜哪一处是浏览器控制入口风险太高（issue #2294）。
    ensure!(
        matching_bindings(text) == 1,
        "Expected one callback binding"
    );
    let path = serde_json::to_string(&control.to_str().context("Non-Unicode control path")?)?;
    // policy 回调名是版本相关的（0.0.24 为 sv，0.0.27 为 wv），必须从原文带过来。
    let replacement = format!(
        // 参数顺序与 0.0.24 一致：policy 回调在前、元数据回调在后、控制文件最后。
        "new {}(r,this.clientApi,()=>{}(this.runtime),this.turnEndedTracker,cppNativeIdentificationReader(this.runtime,{},{},{}))",
        binding.constructor, binding.runtime_getter, binding.policy, binding.runtime_getter, path
    );
    let output = format!(
        "{}{}{}\n{HELPER}",
        &text[..binding.start],
        replacement,
        &text[binding.end..]
    );
    // 运行时自检（issue #2294 第 2 条）：原绑定不得残留，替换串必须落地，
    // 且三个压缩标识符的出现次数必须与替换串里的用法严格对得上。
    ensure!(
        count_occurrences(&output, &original) == 0,
        "Binding rewrite left the original callback in place"
    );
    // 计数只看被改写的正文：HELPER 是追加的独立脚本，里面可能偶然包含
    // 同名子串（例如 `nf` 出现在 `clientInfo` 里），算进来会误报。
    let rewritten = &output[..output.len() - HELPER.len() - 1];
    // 替换串里构造器用一次、policy 回调用一次、元数据回调用两次
    //（`()=>ze(...)` 的调用 + 传给 reader 的实参），所以只有后者 +1。
    // The inserted JSON path is data, not another use of a minified identifier.
    for (name, added) in [
        (&binding.constructor, 0),
        (&binding.runtime_getter, 1),
        (&binding.policy, 0),
    ] {
        ensure!(
            count_occurrences(rewritten, name)
                == count_occurrences(text, name) + added + count_occurrences(&path, name),
            "Binding rewrite changed the {name} occurrence count"
        );
    }
    Ok(output.into_bytes())
}

fn selected_key(descriptor: &Value, root: &Path) -> Result<String> {
    let server = &descriptor["mcpServers"]["cua_repl"];
    let node = PathBuf::from(
        server["command"]
            .as_str()
            .context("Missing native Node command")?,
    );
    let relative = node
        .strip_prefix(root)
        .context("Native Node is outside the runtime cache")?;
    let parts: Vec<_> = relative.components().collect();
    ensure!(parts.len() == 3, "Unexpected native runtime layout");
    let key = parts[0]
        .as_os_str()
        .to_str()
        .context("Invalid runtime key")?;
    ensure!(
        key_valid(key) && relative == Path::new(key).join("bin/node.exe"),
        "Unexpected native runtime layout"
    );
    let env = &server["env"];
    ensure!(
        env["NODE_REPL_NODE_PATH"].as_str().map(Path::new) == Some(node.as_path()),
        "Conflicting native Node selection"
    );
    let worker = root.join(key).join("bin/node_repl.exe");
    ensure!(
        env["CUA_REPL_NODE_REPL_PATH"].as_str().map(Path::new) == Some(worker.as_path()),
        "Conflicting native worker selection"
    );
    let services: Value = serde_json::from_str(
        env["NODE_REPL_TRUSTED_SERVICES"]
            .as_str()
            .context("Missing native service map")?,
    )?;
    ensure!(
        services["browser"] == "@oai/browser-desktop/service",
        "Original browser service is not selected"
    );
    ensure!(
        env["CUA_REPL_ENABLED_SURFACES"].as_str() == Some("browser"),
        "Unsupported native surface selection"
    );
    let args = server["args"]
        .as_array()
        .context("Missing native entry point")?;
    let entry = root.join(key).join(CUA_ENTRY);
    ensure!(
        args.len() == 1 && args[0].as_str().map(Path::new) == Some(entry.as_path()),
        "Unsupported native entry point"
    );
    plain_path(&node)?;
    Ok(key.to_owned())
}

fn discover(paths: &BrowserPaths) -> Result<Option<String>> {
    let plugins = paths
        .codex_home
        .join("plugins/cache/openai-bundled/unified-computer-use");
    plain_path(&plugins)?;
    if !plugins.exists() {
        return Ok(None);
    }
    let mut keys = BTreeSet::new();
    let mut count = 0;
    for entry in fs::read_dir(plugins)? {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        count += 1;
        ensure!(count <= 64, "Too many plugin descriptors");
        let descriptor = entry.path().join(".mcp.json");
        if !descriptor.exists() {
            continue;
        }
        let data: Value = serde_json::from_slice(&read_regular(&descriptor, 1024 * 1024)?)?;
        keys.insert(selected_key(&data, &paths.runtime_root)?);
    }
    ensure!(
        keys.len() <= 1,
        "Ambiguous runtime selection; no cache was modified"
    );
    Ok(keys.into_iter().next())
}

fn prepare(paths: &BrowserPaths, key: &str, contract: &RuntimeContract) -> Result<()> {
    ensure!(key_valid(key), "Invalid runtime key");
    let runtime = paths.runtime_root.join(key);
    let target = runtime.join(SERVICE);
    let _runtime_guards = pin_parents(&target)?;
    // 已知良好版本比哈希；未知版本（issue #2294 的降级通道）在 for_manifest 里
    // 已经确认过结构不变量，这里只再确认文件仍然可读且未被换成链接。
    for (file, expected) in &contract.files {
        match expected {
            FileCheck::Known(expected) => ensure!(
                sha(&read_regular(&runtime.join(file), 128 * 1024 * 1024)?) == *expected,
                UnverifiedRuntime((*file).into())
            ),
            FileCheck::Structural => {
                read_regular(&runtime.join(file), 128 * 1024 * 1024)?;
            }
        }
    }
    let mut current = read_regular(&target, MAX_SERVICE)?;
    let backup_dir = paths.state_root.join(key);
    plain_path(&backup_dir)?;
    fs::create_dir_all(&backup_dir)?;
    let _backup_guards = pin_parents(&backup_dir.join("journal.json"))?;
    let backup = backup_dir.join("original.mjs");
    let journal_path = backup_dir.join("journal.json");
    let control = paths.state_root.join("control.json");
    if journal_path.exists() {
        let (journal, original, recorded_candidate) = recovery_material(paths, key)?;
        let candidate = transform(&original, &control, contract)?;
        if current == recorded_candidate {
            if candidate == recorded_candidate {
                return Ok(());
            }
            // Restore before upgrading the journal, so either journal can recover a crash.
            ensure!(
                read_regular(&target, MAX_SERVICE)? == current,
                "Concurrent adapter upgrade"
            );
            let modified = UNIX_EPOCH
                .checked_add(Duration::new(journal.modified_secs, journal.modified_nanos))
                .context("Invalid recovery timestamp")?;
            atomic_write_with_modified(&target, &original, Some(modified))?;
            current = original;
        }
        ensure!(
            sha(&current) == contract.service_sha,
            "Runtime changed outside Codex++"
        );
    }
    {
        let candidate = transform(&current, &control, contract)?;
        if backup.exists() {
            ensure!(
                read_regular(&backup, MAX_SERVICE)? == current,
                "Unjournaled backup conflict"
            );
        } else {
            write_new(&backup, &current)?;
        }
        let candidate_path = backup_dir.join(format!("candidate-{}.mjs", sha(&candidate)));
        if candidate_path.exists() {
            ensure!(
                read_regular(&candidate_path, MAX_SERVICE)? == candidate,
                "Candidate backup conflict"
            );
        } else {
            write_new(&candidate_path, &candidate)?;
        }
        let modified = fs::metadata(&target)?
            .modified()?
            .duration_since(UNIX_EPOCH)?;
        let journal = Journal {
            schema: if contract.binding.is_some() { 2 } else { 1 },
            original_sha: contract.service_sha.clone(),
            candidate_sha: sha(&candidate),
            modified_secs: modified.as_secs(),
            modified_nanos: modified.subsec_nanos(),
            binding: contract.binding.clone(),
        };
        // Durable original and journal precede any runtime write.
        atomic_write(&journal_path, &serde_json::to_vec(&journal)?)?;
    }
    let (journal, original, candidate) = recovery_material(paths, key)?;
    if current == candidate {
        return Ok(());
    }
    ensure!(
        current == original && sha(&current) == journal.original_sha,
        "Runtime changed outside Codex++; refusing to overwrite"
    );
    ensure!(
        read_regular(&target, MAX_SERVICE)? == current,
        "Concurrent runtime change"
    );
    atomic_write(&target, &candidate)?;
    ensure!(
        read_regular(&target, MAX_SERVICE)? == candidate,
        "Runtime write verification failed"
    );
    Ok(())
}

fn recovery_material(paths: &BrowserPaths, key: &str) -> Result<(Journal, Vec<u8>, Vec<u8>)> {
    ensure!(key_valid(key), "Invalid recovery key");
    let dir = paths.state_root.join(key);
    let journal: Journal = serde_json::from_slice(&read_regular(&dir.join("journal.json"), 4096)?)?;
    let original = read_regular(&dir.join("original.mjs"), MAX_SERVICE)?;
    ensure!(
        journal.candidate_sha.len() == 64
            && journal.candidate_sha.bytes().all(|b| b.is_ascii_hexdigit()),
        "Invalid candidate hash"
    );
    let candidate = read_regular(
        &dir.join(format!("candidate-{}.mjs", journal.candidate_sha)),
        MAX_SERVICE,
    )?;
    let valid_schema = match (journal.schema, &journal.binding) {
        (1, None) => true,
        (2, Some(binding)) => {
            binding.replace(&original, &paths.state_root.join("control.json"))? == candidate
        }
        _ => false,
    };
    ensure!(
        valid_schema && sha(&original) == journal.original_sha
            && journal.candidate_sha == sha(&candidate)
            && journal.modified_nanos < 1_000_000_000,
        "Recovery journal conflicts with verified content"
    );
    Ok((journal, original, candidate))
}

fn restore_all(paths: &BrowserPaths, keep: Option<&str>) -> Result<()> {
    let mut pending = Vec::new();
    let mut guards = Vec::new();
    for entry in fs::read_dir(&paths.state_root)? {
        let entry = entry?;
        let key = entry.file_name().to_string_lossy().to_string();
        if !key_valid(&key) || keep == Some(key.as_str()) {
            continue;
        }
        let dir = entry.path();
        plain_path(&dir)?;
        if !dir.join("journal.json").exists() {
            continue;
        }
        let target = paths.runtime_root.join(&key).join(SERVICE);
        if !target.exists() {
            continue; // Desktop owns cache deletion; never resurrect an obsolete runtime.
        }
        let (journal, original, candidate) = recovery_material(paths, &key)?;
        guards.extend(pin_parents(&target)?);
        let current = read_regular(&target, MAX_SERVICE)?;
        ensure!(
            current == original || current == candidate,
            "External runtime change prevents recovery"
        );
        if current == candidate {
            let modified = UNIX_EPOCH
                .checked_add(Duration::new(journal.modified_secs, journal.modified_nanos))
                .context("Invalid recovery timestamp")?;
            pending.push((target, modified, original, current));
        }
    }
    // Preflight every cache before restoring any, independent of directory enumeration order.
    for (target, modified, original, current) in pending {
        ensure!(
            read_regular(&target, MAX_SERVICE)? == current,
            "Concurrent recovery change"
        );
        atomic_write_with_modified(&target, &original, Some(modified))?;
        ensure!(
            read_regular(&target, MAX_SERVICE)? == original,
            "Recovery verification failed"
        );
    }
    Ok(())
}

/// No runtime operation occurs when this feature has never been enabled.
/// Call only from the owning launcher, never from settings save or status inspection.
pub fn reconcile(paths: &BrowserPaths, enabled: bool) -> Result<BrowserStatus> {
    reconcile_contract(paths, enabled, &RuntimeContract::pinned())
}

fn reconcile_contract(
    paths: &BrowserPaths,
    enabled: bool,
    contract: &RuntimeContract,
) -> Result<BrowserStatus> {
    plain_path(&paths.runtime_root)?;
    plain_path(&paths.state_root)?;
    ensure!(
        !paths.state_root.starts_with(&paths.runtime_root),
        "Backups must be outside the cache"
    );
    if !enabled && !paths.state_root.exists() {
        return Ok(BrowserStatus::new("disabled", "Not configured"));
    }
    fs::create_dir_all(&paths.state_root)?;
    let _guards = pin_parents(&paths.state_root.join("owner.lock"))?;
    let lock_path = paths.state_root.join("owner.lock");
    plain_path(&lock_path)?;
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(lock_path)?;
    lock.try_lock_exclusive()
        .context("Another compatibility transaction is active")?;
    let result = reconcile_locked(paths, enabled, contract);
    if result.is_err() {
        // Fail closed for an already-loaded helper as well as future workers.
        let _ = atomic_write(
            &paths.state_root.join("control.json"),
            br#"{"schema":1,"requireIdentification":false}"#,
        );
    }
    result
}

fn reconcile_locked(
    paths: &BrowserPaths,
    enabled: bool,
    contract: &RuntimeContract,
) -> Result<BrowserStatus> {
    let control = paths.state_root.join("control.json");
    if !enabled {
        atomic_write(&control, br#"{"schema":1,"requireIdentification":false}"#)?;
        restore_all(paths, None)?;
        return Ok(BrowserStatus::new(
            "restored",
            "Service restored; extension identification may remain enabled",
        ));
    }
    let Some(key) = discover(paths)? else {
        atomic_write(&control, br#"{"schema":1,"requireIdentification":false}"#)?;
        return Ok(BrowserStatus::new(
            "waiting_for_runtime",
            "Waiting for a native browser runtime descriptor",
        ));
    };
    // 先解析契约、再恢复：恢复记录的 original SHA 必须与当前契约对得上，
    // 否则适配过的新版本一重启就会把自己的日志判成伪造（issue #2294）。
    // 只有默认（pinned）入口才按运行时自身重新解析；夹具注入的自定义契约必须原样生效。
    let selected = if contract.service_sha == ORIGINAL_SHA {
        resolve_contract(paths, &key)?
    } else {
        contract.clone()
    };
    restore_all(paths, Some(&key))?;
    prepare(paths, &key, &selected)?;
    atomic_write(&control, br#"{"schema":1,"requireIdentification":true}"#)?;
    Ok(BrowserStatus::new(
        "prepared",
        if selected.adaptive {
            "Prepared for an unverified newer native runtime; browser operation is not yet verified"
        } else {
            "Prepared for a new native worker; browser operation is not yet verified"
        },
    ))
}

/// 先按 manifest 结构判定版本世代，再用**实际服务字节**的 SHA 约束它。
/// 这样「manifest 是 0.0.24 但服务文件是别的」这种混搭不会被当成已知良好放行（issue #2294）。
/// 已知良好 → 走原哈希快路径；未知但结构合法 → 降级契约，服务 SHA 取现算值。
fn resolve_contract(paths: &BrowserPaths, key: &str) -> Result<RuntimeContract> {
    ensure!(key_valid(key), "Invalid runtime key");
    let runtime = paths.runtime_root.join(key);
    let manifest = read_regular(&runtime.join(MANIFEST), 1024 * 1024).context(MANIFEST)?;
    let source = if paths.state_root.join(key).join("journal.json").exists() {
        recovery_material(paths, key)?.1
    } else {
        read_regular(&runtime.join(SERVICE), MAX_SERVICE)?
    };
    let service_sha = sha(&source);
    let contract = RuntimeContract::for_manifest(&manifest, &runtime, service_sha.clone())
        .context(MANIFEST)?;
    let contract = if contract.adaptive {
        structural::detect(paths, key)?
    } else {
        contract
    };
    if !contract.adaptive {
        ensure!(
            contract.service_sha == service_sha,
            UnverifiedRuntime("browser service".into())
        );
    }
    if contract.adaptive {
        crate::diagnostic_log::append_diagnostic_log(
            "native_browser.runtime_adapted_unknown",
            json!({"serviceSha": service_sha, "manifestSha": sha(&manifest)}),
        )
        .ok();
    }
    Ok(contract)
}

pub fn read_status() -> BrowserStatus {
    let Ok(paths) = BrowserPaths::current() else {
        return BrowserStatus::new("unsupported", "Windows-only experimental compatibility");
    };
    let path = paths.state_root.join("status.json");
    if fs::metadata(&path)
        .and_then(|meta| meta.modified())
        .ok()
        .and_then(|time| time.elapsed().ok())
        .is_some_and(|age| age > Duration::from_secs(90))
    {
        return BrowserStatus::new(
            "stale",
            "No recent launcher status; browser functionality is unverified",
        );
    }
    read_regular(&path, 8192)
        .and_then(|data| Ok(serde_json::from_slice(&data)?))
        .unwrap_or_else(|_| {
            BrowserStatus::new("not_started", "Waiting for the next Codex++ launcher start")
        })
}

pub struct BrowserMonitor {
    shutdown: tokio::sync::oneshot::Sender<()>,
    task: tokio::task::JoinHandle<()>,
}

impl BrowserMonitor {
    pub async fn stop(self) {
        let _ = self.shutdown.send(());
        // A started blocking filesystem transaction must finish before its owner exits.
        if let Err(error) = self.task.await {
            let _ = crate::diagnostic_log::append_diagnostic_log(
                "native_browser.shutdown_failed",
                json!({"detail": error.to_string()}),
            );
        }
    }
}

/// The singleton launcher owns this task. Existing-instance activation does not start another one.
/// The startup snapshot intentionally requires a launcher restart to apply a saved choice.
pub async fn start_monitor(enabled: bool) -> Option<BrowserMonitor> {
    let paths = BrowserPaths::current().ok()?;
    if !enabled && !paths.state_root.exists() {
        return None;
    }
    match start_monitor_with_contract(paths, enabled, RuntimeContract::pinned()).await {
        Ok(monitor) => Some(monitor),
        Err(error) => {
            // Do not overwrite the active owner's control or status on a second launch.
            let _ = crate::diagnostic_log::append_diagnostic_log(
                "native_browser.owner_refused",
                json!({"detail": error.to_string()}),
            );
            None
        }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct MonitorReceipt {
    schema: u32,
    generation: String,
    state: String,
}

fn write_monitor_receipt(file: &mut File, generation: &str, state: &str) -> Result<()> {
    let bytes = serde_json::to_vec(&MonitorReceipt {
        schema: 1,
        generation: generation.into(),
        state: state.into(),
    })?;
    file.seek(SeekFrom::Start(0))?;
    file.set_len(0)?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    Ok(())
}

fn verify_restored_state(paths: &BrowserPaths) -> Result<()> {
    if !paths.state_root.exists() {
        return Ok(());
    }
    let control = paths.state_root.join("control.json");
    if control.exists() {
        let value: Value = serde_json::from_slice(&read_regular(&control, 1024)?)?;
        ensure!(
            value["schema"] == 1 && value["requireIdentification"] == false,
            "Native browser compatibility is still enabled"
        );
    }
    for entry in fs::read_dir(&paths.state_root)? {
        let key = entry?.file_name().to_string_lossy().to_string();
        if !key_valid(&key) || !paths.state_root.join(&key).join("journal.json").exists() {
            continue;
        }
        let target = paths.runtime_root.join(&key).join(SERVICE);
        if target.exists() {
            let (_, original, _) = recovery_material(paths, &key)?;
            ensure!(
                read_regular(&target, MAX_SERVICE)? == original,
                "Native browser service has not been restored"
            );
        }
    }
    Ok(())
}

fn acquire_monitor_owner(paths: &BrowserPaths) -> Result<File> {
    plain_path(&paths.state_root)?;
    ensure!(
        !paths.state_root.starts_with(&paths.runtime_root),
        "Backups must be outside the cache"
    );
    fs::create_dir_all(&paths.state_root)?;
    let path = paths.state_root.join("monitor.lock");
    let _guards = pin_parents(&path)?;
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(0x1 | 0x2).custom_flags(0x00200000);
    }
    let owner = options.open(&path)?;
    let meta = owner.metadata()?;
    ensure!(meta.is_file(), "Unexpected monitor lock type");
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        ensure!(meta.file_attributes() & 0x400 == 0, "Monitor lock is a reparse point");
    }
    plain_path(&path)?;
    owner.try_lock_exclusive().context("Another native browser monitor is active")?;
    Ok(owner)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativeBrowserShutdown {
    Ready,
    /// The monitor finished and released the lock, but restore did not reach `restored`.
    RestoreFailed,
}

#[derive(Debug)]
pub struct NativeBrowserCleanupStillRunning;

impl std::fmt::Display for NativeBrowserCleanupStillRunning {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "Native browser cleanup is still running; launcher was not terminated"
        )
    }
}

impl std::error::Error for NativeBrowserCleanupStillRunning {}

/// Called after Codex has been stopped, before the manager launches a replacement.
/// Never restores files itself or creates a lock for an older launcher.
pub fn wait_for_monitor_shutdown(timeout: Duration) -> Result<NativeBrowserShutdown> {
    if !cfg!(windows) {
        return Ok(NativeBrowserShutdown::Ready);
    }
    let paths = BrowserPaths::current()?;
    wait_for_monitor_shutdown_at(&paths, timeout)
}

fn wait_for_monitor_shutdown_at(
    paths: &BrowserPaths,
    timeout: Duration,
) -> Result<NativeBrowserShutdown> {
    let path = paths.state_root.join("monitor.lock");
    let _guards = pin_parents(&path)?;
    let mut options = OpenOptions::new();
    options.read(true).write(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(0x1 | 0x2).custom_flags(0x00200000);
    }
    let mut file = match options.open(&path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            verify_restored_state(paths)?;
            return Ok(NativeBrowserShutdown::Ready);
        }
        Err(error) => return Err(error.into()),
    };
    plain_path(&path)?;
    let deadline = std::time::Instant::now() + timeout;
    loop {
        match file.try_lock_exclusive() {
            Ok(()) => {
                ensure!(file.metadata()?.len() <= 1024, "Invalid native cleanup receipt");
                file.seek(SeekFrom::Start(0))?;
                let mut bytes = Vec::new();
                Read::by_ref(&mut file).take(1025).read_to_end(&mut bytes)?;
                let receipt: MonitorReceipt = serde_json::from_slice(&bytes)?;
                ensure!(
                    receipt.schema == 1
                        && uuid::Uuid::parse_str(&receipt.generation).is_ok()
                        && matches!(receipt.state.as_str(), "active" | "blocked" | "restored"),
                    "Invalid native browser cleanup receipt"
                );
                // A completed blocked recovery remains non-fatal upstream. A stale
                // receipt is ready only when read-only disk verification succeeds.
                let restored = verify_restored_state(paths);
                if receipt.state == "blocked" && restored.is_err() {
                    return Ok(NativeBrowserShutdown::RestoreFailed);
                }
                restored.with_context(|| {
                    format!(
                        "Native browser cleanup is incomplete ({}); files were not overwritten",
                        receipt.state
                    )
                })?;
                return Ok(NativeBrowserShutdown::Ready);
            }
            Err(error) if error.kind() == fs2::lock_contended_error().kind() => {
                if std::time::Instant::now() >= deadline {
                    return Err(anyhow::Error::new(NativeBrowserCleanupStillRunning));
                }
                std::thread::sleep(Duration::from_millis(50).min(
                    deadline.saturating_duration_since(std::time::Instant::now()),
                ));
            }
            Err(error) => return Err(error.into()),
        }
    }
}

async fn start_monitor_with_contract(
    paths: BrowserPaths,
    enabled: bool,
    contract: RuntimeContract,
) -> Result<BrowserMonitor> {
    let mut owner = acquire_monitor_owner(&paths)?;
    let generation = uuid::Uuid::new_v4().to_string();
    write_monitor_receipt(&mut owner, &generation, "active")?;
    let initial = monitor_once(paths.clone(), enabled, None, contract.clone()).await;
    if let Ok((status, _)) = &initial {
        let _ = crate::diagnostic_log::append_diagnostic_log(
            "native_browser.compatibility",
            json!(status),
        );
    }
    let (shutdown, mut stopped) = tokio::sync::oneshot::channel();
    let task = tokio::spawn(async move {
        let mut previous = initial.ok();
        let started = std::time::Instant::now();
        loop {
            let delay = if started.elapsed() < Duration::from_secs(30)
                && previous
                    .as_ref()
                    .is_some_and(|(s, _)| s.state == "waiting_for_runtime")
            {
                Duration::from_millis(500)
            } else {
                Duration::from_secs(15)
            };
            tokio::select! {
                _ = &mut stopped => break,
                _ = tokio::time::sleep(delay) => {}
            }
            let outcome = monitor_once(paths.clone(), enabled, previous.clone(), contract.clone()).await;
            if let Ok((status, fingerprint)) = outcome {
                if previous.as_ref().map(|(s, _)| s) != Some(&status) {
                    let _ = crate::diagnostic_log::append_diagnostic_log(
                        "native_browser.compatibility",
                        json!(status),
                    );
                }
                previous = Some((status, fingerprint));
            }
        }
        // Keep lifetime ownership through recovery; a new monitor must not race this restore.
        match monitor_once(paths, false, None, contract).await {
            Ok((status, _)) => {
                if let Err(error) = write_monitor_receipt(&mut owner, &generation, &status.state) {
                    let _ = crate::diagnostic_log::append_diagnostic_log(
                        "native_browser.shutdown_failed",
                        json!({"detail": error.to_string()}),
                    );
                }
                let _ = crate::diagnostic_log::append_diagnostic_log(
                    "native_browser.compatibility",
                    json!(status),
                );
            }
            Err(error) => {
                let _ = crate::diagnostic_log::append_diagnostic_log(
                    "native_browser.shutdown_failed",
                    json!({"detail": error.to_string()}),
                );
            }
        }
        drop(owner);
    });
    Ok(BrowserMonitor { shutdown, task })
}

fn error_status(error: &anyhow::Error) -> BrowserStatus {
    if error.downcast_ref::<UnverifiedRuntime>().is_some() {
        return BrowserStatus::new("runtime_unverified", &error.to_string());
    }
    let retryable = error.chain().any(|cause| {
        cause.downcast_ref::<std::io::Error>().is_some_and(|error| {
            matches!(
                error.kind(),
                std::io::ErrorKind::NotFound | std::io::ErrorKind::WouldBlock
            ) || matches!(error.raw_os_error(), Some(32 | 33))
        }) || cause
            .downcast_ref::<serde_json::Error>()
            .is_some_and(|error| error.is_eof())
    });
    BrowserStatus::new(
        if retryable {
            "waiting_for_runtime"
        } else {
            "blocked"
        },
        &format!("Compatibility refused: {error}"),
    )
}

// A process-local observation cache avoids repeated large-file hashing at idle.
// It is never trusted to authorize a write; reconcile still verifies the full contract.
fn observation(paths: &BrowserPaths) -> Result<String> {
    let mut files = BTreeSet::new();
    let mut keys = BTreeSet::new();
    if let Some(key) = discover(paths)? {
        keys.insert(key);
    }
    let plugin = paths
        .codex_home
        .join("plugins/cache/openai-bundled/unified-computer-use");
    if plugin.exists() {
        for entry in fs::read_dir(plugin)? {
            files.insert(entry?.path().join(".mcp.json"));
        }
    }
    files.insert(paths.state_root.join("control.json"));
    if paths.state_root.exists() {
        for entry in fs::read_dir(&paths.state_root)? {
            let entry = entry?;
            let key = entry.file_name().to_string_lossy().to_string();
            if key_valid(&key) {
                plain_path(&entry.path())?;
                keys.insert(key);
                for file in fs::read_dir(entry.path())? {
                    files.insert(file?.path());
                    ensure!(files.len() <= 1024, "Too many recovery records");
                }
            }
        }
    }
    for key in keys {
        let runtime = paths.runtime_root.join(key);
        files.insert(runtime.join(SERVICE));
        for file in RUNTIME_FILES {
            files.insert(runtime.join(file));
        }
    }
    let mut hash = Sha256::new();
    for path in files {
        plain_path(&path)?;
        hash.update(path.to_string_lossy().as_bytes());
        if !path.exists() {
            hash.update(b"missing");
            continue;
        }
        let file = File::open(&path)?;
        let meta = file.metadata()?;
        ensure!(meta.is_file(), "Unexpected observation path");
        hash.update(format!(
            "{:?}:{:?}:{}",
            meta.created()?,
            meta.modified()?,
            meta.len()
        ));
        #[cfg(windows)]
        {
            use std::os::windows::io::AsRawHandle;
            use windows::Win32::Foundation::HANDLE;
            use windows::Win32::Storage::FileSystem::{
                BY_HANDLE_FILE_INFORMATION, GetFileInformationByHandle,
            };
            let mut identity = BY_HANDLE_FILE_INFORMATION::default();
            unsafe {
                GetFileInformationByHandle(HANDLE(file.as_raw_handle()), &mut identity)?;
            }
            ensure!(
                identity.dwFileAttributes & 0x400 == 0,
                "Reparse observation file"
            );
            hash.update(identity.dwVolumeSerialNumber.to_le_bytes());
            hash.update(identity.nFileIndexHigh.to_le_bytes());
            hash.update(identity.nFileIndexLow.to_le_bytes());
        }
    }
    Ok(format!("{:x}", hash.finalize()))
}

async fn monitor_once(
    paths: BrowserPaths,
    enabled: bool,
    cache: Option<(BrowserStatus, Option<String>)>,
    contract: RuntimeContract,
) -> Result<(BrowserStatus, Option<String>)> {
    tokio::task::spawn_blocking(move || {
        let before = observation(&paths).ok();
        let cached = cache.filter(|(status, observed)| {
            matches!(status.state.as_str(), "prepared" | "restored" | "blocked" | "runtime_unverified")
                && before.is_some()
                && &before == observed
        });
        let (status, fingerprint) = match cached {
            Some((status, _)) => (status, before),
            None => {
                let status =
                    reconcile_contract(&paths, enabled, &contract).unwrap_or_else(|error| error_status(&error));
                let fingerprint = observation(&paths).ok();
                (status, fingerprint)
            }
        };
        if paths.state_root.exists() {
            let _ = atomic_write(
                &paths.state_root.join("status.json"),
                &serde_json::to_vec(&status).unwrap_or_default(),
            );
        }
        (status, fingerprint)
    })
    .await
    .map_err(anyhow::Error::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 两代已登记运行时的原始绑定，作为结构匹配的输入夹具。
    /// 生产代码不再依赖它们的字面量，只依赖 `find_binding` 描述的形状（issue #2294）。
    const ANCHOR: &str = "new nf(r,this.clientApi,()=>ze(this.runtime),this.turnEndedTracker,cD)";
    const CURRENT_ANCHOR: &str =
        "new eh(r,this.clientApi,()=>je(this.runtime),this.turnEndedTracker,sv)";

    /// `plain_path` 拒绝祖先链上含软链的路径（防 junction / symlink 攻击，见该函数注释）。
    /// macOS 上 `/var` 是指向 `/private/var` 的系统软链，而 `tempfile` 默认建在
    /// `/var/folders/...` 下——直接用 `temp.path()` 会让所有测试都撞上这条校验。
    /// 这里 canonicalize 到真实路径，既保留被校验路径的生产语义，又让测试可跨平台运行。
    /// Windows 的 junction 重定向（如被重定向的 TEMP）不在此 helper 的处理范围内，
    /// 那属于 `plain_path` 自身需要收紧的地方。
    pub(super) fn temp_root(temp: &tempfile::TempDir) -> PathBuf {
        let canonical = temp.path().canonicalize().expect("temp dir should canonicalize");
        // Windows 的 canonicalize 会加上 `\\?\` verbatim 前缀。测试构造的路径要参与
        // 字符串形态断言（分隔符风格、路径比较），带上这个前缀会改变语义，所以剥掉。
        #[cfg(windows)]
        {
            let text = canonical.to_string_lossy().to_string();
            if let Some(stripped) = text.strip_prefix(r"\\?\") {
                return PathBuf::from(stripped);
            }
        }
        canonical
    }

    fn paths(temp: &tempfile::TempDir) -> BrowserPaths {
        let root = temp_root(temp);
        BrowserPaths {
            codex_home: root.join("home"),
            runtime_root: root.join("cache"),
            state_root: root.join("state"),
        }
    }

    #[test]
    fn default_does_not_create_or_discover_anything() {
        let temp = tempfile::tempdir().unwrap();
        let paths = paths(&temp);
        assert_eq!(reconcile(&paths, false).unwrap().state, "disabled");
        assert!(!paths.state_root.exists());
    }

    #[test]
    fn missing_runtime_waits_without_enabling() {
        let temp = tempfile::tempdir().unwrap();
        let paths = paths(&temp);
        assert_eq!(
            reconcile(&paths, true).unwrap().state,
            "waiting_for_runtime"
        );
        let control: Value =
            serde_json::from_slice(&fs::read(paths.state_root.join("control.json")).unwrap())
                .unwrap();
        assert_eq!(control["requireIdentification"], false);
    }

    #[test]
    fn unknown_hash_is_rejected_even_with_matching_anchor() {
        assert!(
            transform(
                ANCHOR.as_bytes(),
                Path::new("C:/state/control.json"),
                &RuntimeContract::pinned()
            )
            .is_err()
        );
        assert!(
            transform(
                CURRENT_ANCHOR.as_bytes(),
                Path::new("C:/state/control.json"),
                &RuntimeContract::current()
            )
            .is_err()
        );
        // 只有真实登记的 manifest 字节才能命中哈希快路径；结构合法但未登记的
        // 版本必须走降级通道（下方 `newer_manifest_shape_*` 用例覆盖）。
        assert!(
            RuntimeContract::for_manifest(
                br#"{"name":"@oai/cua-node","version":"0.0.24"}"#,
                Path::new("C:/runtime"),
                CURRENT_ORIGINAL_SHA.into(),
            )
            .is_err(),
            "未登记的 manifest 字节在结构校验缺失组件时必须失败"
        );
    }

    #[test]
    fn unsupported_manifest_does_not_modify_the_browser_service() {
        let temp = tempfile::tempdir().unwrap();
        let (paths, _, service) = synthetic(&temp);
        let before = fs::read(&service).unwrap();
        fs::write(paths.runtime_root.join("0123456789abcdef/manifest.json"), b"unknown").unwrap();
        let error = reconcile(&paths, true).unwrap_err();
        assert!(error.to_string().contains("manifest.json"));
        assert_eq!(fs::read(service).unwrap(), before);
        let control: Value =
            serde_json::from_slice(&fs::read(paths.state_root.join("control.json")).unwrap())
                .unwrap();
        assert_eq!(control["requireIdentification"], false);
    }

    /// issue #2294 主路径：未登记的新版本不再 fail-closed，而是走结构校验后放行。
    #[test]
    fn newer_manifest_shape_is_accepted_and_enables_identification() {
        let temp = tempfile::tempdir().unwrap();
        let (paths, key) = structural::synthetic_runtime(&temp);
        let contract = structural::detect_synthetic(&paths, key).unwrap();
        let service = paths.runtime_root.join(key).join(SERVICE);
        let dir = paths.codex_home.join("plugins/cache/openai-bundled/unified-computer-use/test");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join(".mcp.json"), serde_json::to_vec(&descriptor(&paths.runtime_root, key)).unwrap()).unwrap();
        let original = fs::read(&service).unwrap();
        let status = reconcile_contract(&paths, true, &contract).unwrap();
        assert_eq!(status.state, "prepared", "{}", status.detail);
        let candidate = String::from_utf8(fs::read(&service).unwrap()).unwrap();
        assert!(candidate.contains("cppNativeIdentificationReader"), "{candidate}");
        assert_ne!(fs::read(&service).unwrap(), original);
        let control: Value =
            serde_json::from_slice(&fs::read(paths.state_root.join("control.json")).unwrap())
                .unwrap();
        // 这一条正是 #2209 的判据：白名单过窄时这里会被写成 false，回到云端策略。
        assert_eq!(control["requireIdentification"], true);
        assert_eq!(reconcile(&paths, false).unwrap().state, "restored");
        assert_eq!(fs::read(&service).unwrap(), original);
    }

    /// 回归：结构校验必须真的校验结构。挖掉 cua-repl 入口后 for_manifest 必须 Err。
    #[test]
    fn missing_cua_repl_entry_rejects_an_otherwise_valid_manifest() {
        let temp = tempfile::tempdir().unwrap();
        let (paths, _, _) = synthetic_with(&temp, "0.0.27");
        let key = "0123456789abcdef";
        let runtime = paths.runtime_root.join(key);
        let manifest = fs::read(runtime.join(MANIFEST)).unwrap();
        let service_sha = sha(&fs::read(runtime.join(SERVICE)).unwrap());
        // 结构齐备时接受
        assert!(RuntimeContract::for_manifest(&manifest, &runtime, service_sha.clone()).is_ok());
        // 挖掉入口后必须拒绝，且不得降级成「接受」
        fs::remove_file(runtime.join(CUA_ENTRY)).unwrap();
        let error = RuntimeContract::for_manifest(&manifest, &runtime, service_sha)
            .err()
            .expect("缺少 cua-repl 入口时必须拒绝");
        assert!(error.to_string().contains(CUA_ENTRY), "{error}");
    }

    #[test]
    fn manifest_requires_fields_version_floor_and_parsable_versions() {
        let temp = tempfile::tempdir().unwrap();
        let (paths, _, _) = synthetic_with(&temp, "0.0.27");
        let runtime = paths.runtime_root.join("0123456789abcdef");
        let sha = sha(&fs::read(runtime.join(SERVICE)).unwrap());
        let accepts = |manifest: &str| {
            RuntimeContract::for_manifest(manifest.as_bytes(), &runtime, sha.clone()).is_ok()
        };
        // 缺失必要字段
        assert!(!accepts(r#"{"name":"@oai/cua-node"}"#));
        assert!(!accepts(r#"{"version":"0.0.27"}"#));
        assert!(!accepts(r#"{"name":null,"version":"0.0.27"}"#));
        // 非 JSON / 非对象
        assert!(!accepts("not json"));
        assert!(!accepts("[1,2,3]"));
        // 版本号必须可解析且不低于 0.0.11
        assert!(!accepts(r#"{"name":"x","version":"0.0.10"}"#));
        assert!(!accepts(r#"{"name":"x","version":"0.0"}"#));
        assert!(!accepts(r#"{"name":"x","version":"latest"}"#));
        assert!(!accepts(r#"{"name":"x","version":27}"#));
        // 新版、预发布后缀、v 前缀、额外字段都必须照常接受
        for version in ["0.0.27", "0.0.11", "v1.0.0", "0.1.0-beta.1", " 2.0.0 "] {
            assert!(
                accepts(&format!(r#"{{"name":"x","version":"{version}","extra":1}}"#)),
                "版本 {version} 应被接受"
            );
        }
        assert!(accepts(r#"{"runtime_archive_version":"0.0.27/20260927214556-b77d38801cca","runtime_archive_name":"cua-node-0.0.27-20260927214556-b77d38801cca-windows-x64.zip"}"#));
        assert!(!accepts(r#"{"runtime_archive_version":"0.0.10/build","runtime_archive_name":"cua-node-0.0.10-build-windows-x64.zip"}"#));
        assert!(!accepts(r#"{"runtime_archive_version":"0.0.27/build","runtime_archive_name":"cua-node-0.0.28-build-windows-x64.zip"}"#));
        assert!(!accepts(r#"{"runtime_archive_version":"latest/build","runtime_archive_name":"cua-node-latest-build-windows-x64.zip"}"#));
    }

    #[test]
    fn plain_path_rejects_symlinked_ancestors_and_accepts_plain_directories() {
        // 这条校验此前没有任何测试覆盖：既挡住了正常用法（macOS 的 /var 系统软链），
        // 又没有回归保护。这里补上两侧——真软链必须被拒，普通目录必须通过。
        let temp = tempfile::tempdir().unwrap();
        let root = temp_root(&temp);

        let plain = root.join("plain");
        fs::create_dir_all(&plain).unwrap();
        assert!(plain_path(&plain).is_ok(), "普通目录应通过");
        assert!(plain_path(&plain.join("state")).is_ok(), "尚不存在的子路径应通过");

        // 路径本身是软链
        #[cfg(unix)]
        {
            let target = root.join("target");
            fs::create_dir_all(&target).unwrap();
            let link = root.join("link");
            std::os::unix::fs::symlink(&target, &link).unwrap();
            let error = plain_path(&link).unwrap_err();
            assert!(error.to_string().contains("Linked paths"), "{error}");

            // 祖先链上有软链（等价于 macOS 的 /var 情形，必须一并拒绝）
            let nested = link.join("state");
            let error = plain_path(&nested).unwrap_err();
            assert!(error.to_string().contains("Linked paths"), "{error}");

            // 拒绝的是「路径里有软链」，不是「指向的目标不可用」：
            // 走真实路径访问同一目录应当通过。
            assert!(plain_path(&target.join("state")).is_ok());
        }

        // 相对路径与含 .. 的路径
        assert!(plain_path(Path::new("relative/path")).is_err(), "相对路径应被拒");
        assert!(
            plain_path(&root.join("a").join("..").join("b")).is_err(),
            "含 .. 的路径应被拒"
        );
    }

    #[test]
    fn control_path_substrings_are_not_identifier_drift() {
        let text = format!("fixture;{ANCHOR};original");
        let contract = RuntimeContract {
            service_sha: sha(text.as_bytes()),
            files: vec![],
            adaptive: false,
            // 手工构造：本用例只验证控制文件路径不参与标识符漂移判定，
            // 不经结构探测，故无 binding（issue #2378 新增字段）。
            binding: None,
        };
        let output = transform(
            text.as_bytes(), Path::new("C:/conflict/nf-ze-cD/control.json"), &contract,
        ).unwrap();
        let rewritten = String::from_utf8(output).unwrap();
        assert!(rewritten.contains("C:/conflict/nf-ze-cD/control.json"));
        assert!(rewritten.contains("cppNativeIdentificationReader(this.runtime,cD,ze,"));
    }

    #[test]
    fn binding_requires_unique_anchor_and_preserves_other_code() {
        let path = Path::new("C:/unicode-\u{4e2d}/control.json");
        for source in ["no binding".to_string(), ANCHOR.repeat(2)] {
            assert!(
                transform_binding(source.as_bytes(), path, &RuntimeContract::pinned()).is_err()
            );
        }
        let source = format!("prefix;{ANCHOR};suffix");
        let output = String::from_utf8(
            transform_binding(source.as_bytes(), path, &RuntimeContract::pinned()).unwrap(),
        )
        .unwrap();
        assert!(output.starts_with("prefix;new nf("));
        assert!(output.contains(";suffix\n"));
        assert!(output.ends_with(HELPER));
        assert!(!output.contains("turn_id:"));
    }

    #[test]
    fn current_binding_uses_current_metadata_and_policy_callback() {
        let path = Path::new("C:/state/control.json");
        let contract = RuntimeContract::current();
        // 结构匹配不认压缩名，所以 0.0.11 的锚点在 0.0.24 契约下同样能改；
        // 唯一性才是拒绝条件。
        for source in ["no binding".to_owned(), CURRENT_ANCHOR.repeat(2), ANCHOR.repeat(2)] {
            assert!(transform_binding(source.as_bytes(), path, &contract).is_err());
        }
        let source = format!("prefix;{CURRENT_ANCHOR};suffix");
        let output =
            String::from_utf8(transform_binding(source.as_bytes(), path, &contract).unwrap())
                .unwrap();
        // policy 回调 `sv` 与元数据回调 `je` 都必须按原文回填。
        assert!(output.contains("new eh(r,this.clientApi,()=>je(this.runtime),this.turnEndedTracker,cppNativeIdentificationReader(this.runtime,sv,je,"));
        assert!(output.ends_with(HELPER));
        assert!(output.contains(";suffix\n"));
    }

    /// issue #2294：0.0.27 把构造器/回调名整批换新（`eh/je/sv` → `uh/We/wv`）。
    /// 旧实现写死字面锚点，一次发版即失灵；结构匹配必须吃下这一代与以后各代。
    const NEW_ANCHOR: &str =
        "new uh(r,this.clientApi,()=>We(this.runtime),this.turnEndedTracker,wv)";

    #[test]
    fn newer_binding_uses_structural_match_and_carries_policy_callback() {
        let path = Path::new("C:/state/control.json");
        let contract = RuntimeContract::adapted("0".repeat(64).into());
        // 旧的两代锚点在这种契约下同样必须可用（结构不认名字）。
        for source in [ANCHOR.to_owned(), CURRENT_ANCHOR.to_owned()] {
            assert!(
                transform_binding(source.as_bytes(), path, &contract).is_ok(),
                "结构匹配应接受已知各代锚点：{source}"
            );
        }
        let source = format!("prefix;{NEW_ANCHOR};suffix");
        let output =
            String::from_utf8(transform_binding(source.as_bytes(), path, &contract).unwrap())
                .unwrap();
        // 第三个捕获组（policy 回调名 wv）必须原样带进替换串，元数据回调 We 也一样。
        assert!(
            output.contains(
                "new uh(r,this.clientApi,()=>We(this.runtime),this.turnEndedTracker,cppNativeIdentificationReader(this.runtime,wv,We,"
            ),
            "{output}"
        );
        assert!(output.starts_with("prefix;new uh("));
        assert!(output.contains(";suffix\n"));
        assert!(output.ends_with(HELPER));
        // 自检口径：构造器与 policy 回调计数不变，元数据回调因多了一次实参而 +1。
        for (name, added) in [("uh", 0), ("We", 1), ("wv", 0)] {
            assert_eq!(
                count_occurrences(&output, name),
                count_occurrences(&source, name) + added,
                "{name}"
            );
        }
    }

    #[test]
    fn structural_binding_still_rejects_ambiguity_and_conflicts() {
        let path = Path::new("C:/state/control.json");
        let contract = RuntimeContract::adapted("0".repeat(64).into());
        // 两处同类绑定：不能猜一个改。
        assert!(
            transform_binding(
                format!("{NEW_ANCHOR};{CURRENT_ANCHOR}").as_bytes(),
                path,
                &contract
            )
            .is_err()
        );
        // 完全不成形
        for source in ["no binding", "new uh(r,this.clientApi)", "this.turnEndedTracker"] {
            assert!(transform_binding(source.as_bytes(), path, &contract).is_err());
        }
        // 已经适配过：拒绝二次注入。
        assert!(
            transform_binding(
                format!("cppNativeIdentificationReader(this.runtime,wv,We,x)").as_bytes(),
                path,
                &contract
            )
            .is_err()
        );
        // 参数名不被换掉（否则会改到别的构造函数或方法调用上）。
        assert!(
            transform_binding(
                "new uh(record,this.clientApi,()=>We(this.runtime),this.turnEndedTracker,wv)"
                    .as_bytes(),
                path,
                &contract
            )
            .is_err()
        );
    }

    fn descriptor(root: &Path, key: &str) -> Value {
        let node = root.join(key).join("bin/node.exe");
        json!({"mcpServers":{"cua_repl":{
            "command":node, "args":[root.join(key).join("bin/node_modules/@oai/cua-repl/bin/cua-repl.mjs")],
            "env":{"NODE_REPL_NODE_PATH":node,"CUA_REPL_NODE_REPL_PATH":root.join(key).join("bin/node_repl.exe"),"NODE_REPL_TRUSTED_SERVICES":"{\"browser\":\"@oai/browser-desktop/service\"}",
                "CUA_REPL_ENABLED_SURFACES":"browser"}
        }}})
    }

    #[test]
    fn discovery_checks_native_backend_and_paths() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp_root(&temp).join("cache");
        let mut data = descriptor(&root, "0123456789abcdef");
        assert_eq!(selected_key(&data, &root).unwrap(), "0123456789abcdef");
        data["mcpServers"]["cua_repl"]["env"]["NODE_REPL_TRUSTED_SERVICES"] =
            json!("{\"browser\":\"other/backend\"}");
        assert!(selected_key(&data, &root).is_err());
        assert!(selected_key(&descriptor(&root, "../escape"), &root).is_err());
        assert!(selected_key(&descriptor(&temp_root(&temp), "0123456789abcdef"), &root).is_err());
    }

    #[cfg(windows)]
    #[test]
    fn windows_generated_descriptor_accepts_backslash_paths() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp_root(&temp).join("OpenAI/Codex/runtimes/cua_node");
        let key = "0123456789abcdef";
        let base = format!(r"{}\{key}", root.display().to_string().replace('/', "\\"));
        // Desktop writes backslashes independently of our PathBuf joins.
        let data = json!({"mcpServers":{"cua_repl":{
            "command":format!(r"{base}\bin\node.exe"),
            "args":[format!(r"{base}\bin\node_modules\@oai\cua-repl\bin\cua-repl.mjs")],
            "env":{
                "NODE_REPL_NODE_PATH":format!(r"{base}\bin\node.exe"),
                "CUA_REPL_NODE_REPL_PATH":format!(r"{base}\bin\node_repl.exe"),
                "NODE_REPL_TRUSTED_SERVICES":"{\"browser\":\"@oai/browser-desktop/service\"}",
                "CUA_REPL_ENABLED_SURFACES":"browser"
            }
        }}});
        assert_eq!(selected_key(&data, &root).unwrap(), key);
    }

    #[cfg(windows)]
    #[test]
    fn windows_descriptor_accepts_independent_separator_styles() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp_root(&temp).join("OpenAI/Codex/runtimes/cua_node");
        let key = "0123456789abcdef";
        let fields = [
            "/mcpServers/cua_repl/command",
            "/mcpServers/cua_repl/env/NODE_REPL_NODE_PATH",
            "/mcpServers/cua_repl/env/CUA_REPL_NODE_REPL_PATH",
            "/mcpServers/cua_repl/args/0",
        ];
        for mask in 0..16 {
            let mut data = descriptor(&root, key);
            for (index, field) in fields.iter().enumerate() {
                let value = data.pointer_mut(field).unwrap();
                let path = value.as_str().unwrap().replace('\\', "/");
                *value = json!(if mask & (1 << index) == 0 {
                    path
                } else {
                    path.replace('/', "\\")
                });
            }
            assert_eq!(selected_key(&data, &root).unwrap(), key, "mask {mask}");
        }
    }

    #[test]
    fn descriptor_rejects_conflicting_or_malformed_runtime_paths() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp_root(&temp).join("cache");
        let key = "0123456789abcdef";
        for field in [
            "/mcpServers/cua_repl/command",
            "/mcpServers/cua_repl/env/NODE_REPL_NODE_PATH",
            "/mcpServers/cua_repl/env/CUA_REPL_NODE_REPL_PATH",
            "/mcpServers/cua_repl/args/0",
        ] {
            for invalid in [
                Value::Null,
                json!(42),
                json!(""),
                json!("bin/node_repl.exe"),
                json!(root.join("fedcba9876543210/bin/node_repl.exe")),
                json!(root.join(key).join("bin/../bin/node_repl.exe")),
            ] {
                let mut data = descriptor(&root, key);
                *data.pointer_mut(field).unwrap() = invalid;
                assert!(selected_key(&data, &root).is_err(), "{field}");
            }
        }
    }

    #[test]
    fn ambiguous_descriptors_refuse_enablement() {
        let temp = tempfile::tempdir().unwrap();
        let paths = paths(&temp);
        for (version, key) in [("one", "0123456789abcdef"), ("two", "fedcba9876543210")] {
            let dir = paths
                .codex_home
                .join("plugins/cache/openai-bundled/unified-computer-use")
                .join(version);
            fs::create_dir_all(&dir).unwrap();
            fs::write(
                dir.join(".mcp.json"),
                serde_json::to_vec(&descriptor(&paths.runtime_root, key)).unwrap(),
            )
            .unwrap();
        }
        assert!(reconcile(&paths, true).is_err());
        assert!(
            !fs::read_to_string(paths.state_root.join("control.json"))
                .unwrap()
                .contains(":true")
        );
    }

    #[test]
    fn recovery_rejects_forged_journal_and_backup() {
        let temp = tempfile::tempdir().unwrap();
        let paths = paths(&temp);
        let dir = paths.state_root.join("0123456789abcdef");
        let target = paths.runtime_root.join("0123456789abcdef").join(SERVICE);
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        fs::write(target, b"unrecognized service").unwrap();
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("journal.json"), br#"{"schema":1,"originalSha":"fake","candidateSha":"fake","modifiedSecs":0,"modifiedNanos":0}"#).unwrap();
        fs::write(dir.join("original.mjs"), ANCHOR).unwrap();
        assert!(reconcile(&paths, false).is_err());
    }

    #[test]
    fn concurrent_owner_is_refused() {
        let temp = tempfile::tempdir().unwrap();
        let paths = paths(&temp);
        fs::create_dir_all(&paths.state_root).unwrap();
        let file = File::create(paths.state_root.join("owner.lock")).unwrap();
        file.try_lock_exclusive().unwrap();
        assert!(reconcile(&paths, true).is_err());
        assert!(!paths.state_root.join("control.json").exists());
    }

    #[test]
    fn backup_must_be_outside_runtime_cache() {
        let temp = tempfile::tempdir().unwrap();
        let mut paths = paths(&temp);
        paths.state_root = paths.runtime_root.join("backup");
        assert!(reconcile(&paths, true).is_err());
    }

    #[test]
    fn atomic_replace_and_timestamp_roundtrip() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp_root(&temp).join("value");
        write_new(&path, b"original").unwrap();
        assert!(write_new(&path, b"collision").is_err());
        atomic_write(&path, b"candidate").unwrap();
        assert_eq!(read_regular(&path, 50).unwrap(), b"candidate");
        let time = UNIX_EPOCH + Duration::new(1_789_145_796, 123_456_700);
        atomic_write_with_modified(&path, b"original", Some(time)).unwrap();
        assert_eq!(read_regular(&path, 50).unwrap(), b"original");
        assert_eq!(fs::metadata(path).unwrap().modified().unwrap(), time);
    }

    #[cfg(windows)]
    #[test]
    fn failed_atomic_restore_preserves_target_bytes_and_timestamp() {
        use std::os::windows::fs::OpenOptionsExt;
        let temp = tempfile::tempdir().unwrap();
        let path = temp_root(&temp).join("service.mjs");
        write_new(&path, b"candidate").unwrap();
        let before = fs::metadata(&path).unwrap().modified().unwrap();
        let restored = UNIX_EPOCH + Duration::new(1_789_145_796, 123_456_700);
        let held = OpenOptions::new()
            .read(true)
            .share_mode(0x1)
            .open(&path)
            .unwrap();
        assert!(atomic_write_with_modified(&path, b"original", Some(restored)).is_err());
        assert_eq!(fs::read(&path).unwrap(), b"candidate");
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), before);
        assert_eq!(fs::read_dir(temp_root(&temp)).unwrap().count(), 1);
        drop(held);
        atomic_write_with_modified(&path, b"original", Some(restored)).unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"original");
        assert_eq!(fs::metadata(path).unwrap().modified().unwrap(), restored);
    }

    /// 合成运行时夹具。`version` 决定它落在哈希快路径还是结构降级通道：
    /// 夹具 manifest 的 SHA 永远不等于登记的 `CURRENT_MANIFEST_SHA`，
    /// 所以结构校验必须真的通过，`prepare` 才会被调用。
    fn synthetic_with(temp: &tempfile::TempDir, version: &str) -> (BrowserPaths, RuntimeContract, PathBuf) {
        let paths = paths(temp);
        let key = "0123456789abcdef";
        let service = paths.runtime_root.join(key).join(SERVICE);
        let original = format!("fixture;{ANCHOR};original");
        fs::create_dir_all(service.parent().unwrap()).unwrap();
        fs::write(&service, &original).unwrap();
        let contract = RuntimeContract {
            service_sha: sha(original.as_bytes()),
            files: vec![("bin/node.exe", FileCheck::Known(sha(b"fixture-node")))],
            adaptive: false,
            binding: None,
        };
        for file in RUNTIME_FILES {
            let path = paths.runtime_root.join(key).join(file);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(
                &path,
                match file {
                    MANIFEST => format!(r#"{{"name":"@oai/cua-node","version":"{version}"}}"#),
                    "bin/node.exe" => "fixture-node".into(),
                    other => format!("fixture:{other}"),
                },
            )
            .unwrap();
        }
        let dir = paths
            .codex_home
            .join("plugins/cache/openai-bundled/unified-computer-use/test");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join(".mcp.json"),
            serde_json::to_vec(&descriptor(&paths.runtime_root, key)).unwrap(),
        )
        .unwrap();
        (paths, contract, service)
    }

    fn synthetic(temp: &tempfile::TempDir) -> (BrowserPaths, RuntimeContract, PathBuf) {
        synthetic_with(temp, "0.0.24")
    }

    #[test]
    fn structural_journal_restores_without_a_runtime_or_current_profile() {
        let temp = tempfile::tempdir().unwrap();
        let (paths, mut contract, service) = synthetic(&temp);
        let original = fs::read(&service).unwrap();
        let text = std::str::from_utf8(&original).unwrap();
        let start = text.rfind("cD)").unwrap();
        contract.binding = Some(structural::Binding {
            start, end: start + 2, policy: "cD".into(), metadata: "ze".into(),
            contract_sha: "eae1b49427aebf3ed3d1119de1c303125c4f78b3b0ca644f055ed07ec2c2be30".into(),
        });
        let modified = fs::metadata(&service).unwrap().modified().unwrap();
        reconcile_contract(&paths, true, &contract).unwrap();
        let journal = paths.state_root.join("0123456789abcdef/journal.json");
        let data: Value = serde_json::from_slice(&fs::read(&journal).unwrap()).unwrap();
        assert_eq!(data["schema"], 2);
        fs::remove_file(paths.runtime_root.join("0123456789abcdef/bin/node.exe")).unwrap();
        assert_eq!(reconcile(&paths, false).unwrap().state, "restored");
        assert_eq!(fs::read(&service).unwrap(), original);
        assert_eq!(fs::metadata(&service).unwrap().modified().unwrap(), modified);
        // Hash-matching files alone do not authorize an altered transform record.
        let mut bad = data;
        bad["binding"]["metadata"] = json!("wrongMetadata");
        fs::write(&journal, serde_json::to_vec(&bad).unwrap()).unwrap();
        assert!(reconcile(&paths, false).is_err());
        assert_eq!(fs::read(&service).unwrap(), original);
    }

    #[test]
    fn structural_journal_refuses_external_changes_and_deleted_cache_resurrection() {
        let temp = tempfile::tempdir().unwrap();
        let (paths, mut contract, service) = synthetic(&temp);
        let original = fs::read(&service).unwrap();
        let start = std::str::from_utf8(&original).unwrap().rfind("cD)").unwrap();
        contract.binding = Some(structural::Binding {
            start, end: start + 2, policy: "cD".into(), metadata: "ze".into(),
            contract_sha: "5bf64da3b8386af46a6fb2c2d8829a507130ba9896237a813b8e04c4ce8eb515".into(),
        });
        reconcile_contract(&paths, true, &contract).unwrap();
        fs::write(&service, b"external edit").unwrap();
        assert!(reconcile(&paths, false).is_err());
        assert_eq!(fs::read(&service).unwrap(), b"external edit");
        fs::remove_file(&service).unwrap();
        reconcile(&paths, false).unwrap();
        assert!(!service.exists());
    }

    #[test]
    fn synthetic_transaction_and_rebuilt_generation_timestamp() {
        let temp = tempfile::tempdir().unwrap();
        let (paths, contract, service) = synthetic(&temp);
        let original = fs::read(&service).unwrap();
        assert_eq!(
            reconcile_contract(&paths, true, &contract).unwrap().state,
            "prepared"
        );
        let candidate = fs::read(&service).unwrap();
        reconcile_contract(&paths, true, &contract).unwrap();
        assert_eq!(fs::read(&service).unwrap(), candidate);
        reconcile_contract(&paths, false, &contract).unwrap();
        assert_eq!(fs::read(&service).unwrap(), original);
        let rebuilt_time = UNIX_EPOCH + Duration::new(1_789_145_800, 700);
        File::options()
            .write(true)
            .open(&service)
            .unwrap()
            .set_modified(rebuilt_time)
            .unwrap();
        // A disabled reconcile must not overwrite an already-original generation's timestamp.
        reconcile_contract(&paths, false, &contract).unwrap();
        assert_eq!(
            fs::metadata(&service).unwrap().modified().unwrap(),
            rebuilt_time
        );
        reconcile_contract(&paths, true, &contract).unwrap();
        reconcile_contract(&paths, false, &contract).unwrap();
        assert_eq!(
            fs::metadata(&service).unwrap().modified().unwrap(),
            rebuilt_time
        );
    }

    #[test]
    fn stored_candidate_allows_adapter_upgrade_without_overwriting_external_changes() {
        let temp = tempfile::tempdir().unwrap();
        let (paths, contract, service) = synthetic(&temp);
        reconcile_contract(&paths, true, &contract).unwrap();
        let key = "0123456789abcdef";
        let (mut journal, original, current_candidate) =
            recovery_material(&paths, key).unwrap();
        let old_candidate = [
            current_candidate.as_slice(),
            b"\n// previous adapter revision\n",
        ]
        .concat();
        journal.candidate_sha = sha(&old_candidate);
        let backup = paths.state_root.join(key);
        fs::write(
            backup.join(format!("candidate-{}.mjs", journal.candidate_sha)),
            &old_candidate,
        )
        .unwrap();
        fs::write(
            backup.join("journal.json"),
            serde_json::to_vec(&journal).unwrap(),
        )
        .unwrap();
        fs::write(&service, &old_candidate).unwrap();
        reconcile_contract(&paths, true, &contract).unwrap();
        assert_eq!(fs::read(&service).unwrap(), current_candidate);
        fs::write(&service, b"third-party change").unwrap();
        assert!(reconcile_contract(&paths, false, &contract).is_err());
        assert_eq!(fs::read(&service).unwrap(), b"third-party change");
        fs::write(&service, current_candidate).unwrap();
        reconcile_contract(&paths, false, &contract).unwrap();
        assert_eq!(fs::read(service).unwrap(), original);
    }

    #[test]
    fn all_restores_are_preflighted_before_any_runtime_write() {
        let temp = tempfile::tempdir().unwrap();
        let (paths, contract, service) = synthetic(&temp);
        reconcile_contract(&paths, true, &contract).unwrap();
        let candidate = fs::read(&service).unwrap();
        let other = "fedcba9876543210";
        let other_backup = paths.state_root.join(other);
        fs::create_dir_all(&other_backup).unwrap();
        for entry in fs::read_dir(paths.state_root.join("0123456789abcdef")).unwrap() {
            let entry = entry.unwrap();
            fs::copy(entry.path(), other_backup.join(entry.file_name())).unwrap();
        }
        let other_service = paths.runtime_root.join(other).join(SERVICE);
        fs::create_dir_all(other_service.parent().unwrap()).unwrap();
        fs::write(&other_service, b"external change").unwrap();
        assert!(reconcile_contract(&paths, false, &contract).is_err());
        assert_eq!(fs::read(&service).unwrap(), candidate);
        let control: Value =
            serde_json::from_slice(&fs::read(paths.state_root.join("control.json")).unwrap())
                .unwrap();
        assert_eq!(control["requireIdentification"], false);
        fs::write(&other_service, &candidate).unwrap();
        let journal_path = other_backup.join("journal.json");
        let mut journal: Journal =
            serde_json::from_slice(&fs::read(&journal_path).unwrap()).unwrap();
        journal.modified_secs = u64::MAX;
        fs::write(&journal_path, serde_json::to_vec(&journal).unwrap()).unwrap();
        assert!(reconcile_contract(&paths, false, &contract).is_err());
        assert_eq!(fs::read(&service).unwrap(), candidate);
        assert_eq!(fs::read(&other_service).unwrap(), candidate);
    }

    #[test]
    fn changed_entry_component_is_refused_before_service_write() {
        let temp = tempfile::tempdir().unwrap();
        let (paths, contract, service) = synthetic(&temp);
        let original = fs::read(&service).unwrap();
        fs::write(
            paths.runtime_root.join("0123456789abcdef/bin/node.exe"),
            b"unknown-node",
        )
        .unwrap();
        assert!(reconcile_contract(&paths, true, &contract).is_err());
        assert_eq!(fs::read(&service).unwrap(), original);
    }

    #[test]
    fn interrupted_backup_stage_can_be_resumed_and_tampering_is_refused() {
        let temp = tempfile::tempdir().unwrap();
        let (paths, contract, service) = synthetic(&temp);
        let dir = paths.state_root.join("0123456789abcdef");
        fs::create_dir_all(&dir).unwrap();
        fs::copy(&service, dir.join("original.mjs")).unwrap();
        reconcile_contract(&paths, true, &contract).unwrap();
        let candidate = fs::read(&service).unwrap();
        let journal: Journal =
            serde_json::from_slice(&fs::read(dir.join("journal.json")).unwrap()).unwrap();
        fs::write(
            dir.join(format!("candidate-{}.mjs", journal.candidate_sha)),
            b"corrupt backup",
        )
        .unwrap();
        assert!(reconcile_contract(&paths, false, &contract).is_err());
        assert_eq!(fs::read(service).unwrap(), candidate);
    }

    #[cfg(windows)]
    #[test]
    fn pinned_parent_cannot_be_renamed_during_transaction() {
        let temp = tempfile::tempdir().unwrap();
        let parent = temp_root(&temp).join("parent");
        let destination = temp_root(&temp).join("renamed");
        fs::create_dir(&parent).unwrap();
        let guards = pin_parents(&parent.join("service.mjs")).unwrap();
        assert!(fs::rename(&parent, &destination).is_err());
        drop(guards);
        fs::rename(&parent, &destination).unwrap();
    }

    #[test]
    fn removed_cache_does_not_require_obsolete_recovery_records() {
        let temp = tempfile::tempdir().unwrap();
        let paths = paths(&temp);
        let dir = paths.state_root.join("0123456789abcdef");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("journal.json"),
            b"obsolete record without a target",
        )
        .unwrap();
        assert_eq!(reconcile(&paths, false).unwrap().state, "restored");
    }

    #[test]
    fn transient_generation_errors_retry_but_unknown_contracts_do_not() {
        let missing = anyhow::Error::from(std::io::Error::from(std::io::ErrorKind::NotFound));
        assert_eq!(error_status(&missing).state, "waiting_for_runtime");
        let partial = serde_json::from_str::<Value>("{").unwrap_err();
        assert_eq!(error_status(&partial.into()).state, "waiting_for_runtime");
        assert_eq!(
            error_status(&anyhow::anyhow!("Unsupported runtime hash")).state,
            "blocked"
        );
    }

    #[tokio::test]
    async fn idle_observation_does_not_rewrite_control_or_reconcile() {
        let temp = tempfile::tempdir().unwrap();
        let (paths, contract, service) = synthetic(&temp);
        reconcile_contract(&paths, true, &contract).unwrap();
        let control = paths.state_root.join("control.json");
        let modified = fs::metadata(&control).unwrap().modified().unwrap();
        let observed = observation(&paths).unwrap();
        let cached = Some((
            BrowserStatus::new("prepared", "fixture"),
            Some(observed.clone()),
        ));
        let (status, after) =
            monitor_once(paths.clone(), true, cached, RuntimeContract::pinned()).await.unwrap();
        assert_eq!(status.state, "prepared"); // A full pinned-contract reconcile would reject this fixture.
        assert_eq!(after.as_deref(), Some(observed.as_str()));
        assert_eq!(fs::metadata(control).unwrap().modified().unwrap(), modified);
        fs::write(service, b"external change").unwrap();
        assert_ne!(observation(&paths).unwrap(), observed);
    }

    #[tokio::test]
    async fn monitor_stop_restores_original_content_timestamp_and_control() {
        let temp = tempfile::tempdir().unwrap();
        let (paths, contract, service) = synthetic(&temp);
        let original = fs::read(&service).unwrap();
        let modified = fs::metadata(&service).unwrap().modified().unwrap();
        let monitor = start_monitor_with_contract(paths.clone(), true, contract.clone()).await.unwrap();
        assert_ne!(fs::read(&service).unwrap(), original);
        monitor.stop().await;
        assert_eq!(sha(&fs::read(&service).unwrap()), sha(&original));
        assert_eq!(fs::metadata(&service).unwrap().modified().unwrap(), modified);
        let control: Value =
            serde_json::from_slice(&fs::read(paths.state_root.join("control.json")).unwrap()).unwrap();
        assert_eq!(control["requireIdentification"], false);
        let status: BrowserStatus =
            serde_json::from_slice(&fs::read(paths.state_root.join("status.json")).unwrap()).unwrap();
        assert_eq!(status.state, "restored");
    }

    #[tokio::test]
    async fn second_monitor_cannot_disable_or_restore_the_active_owner() {
        let temp = tempfile::tempdir().unwrap();
        let (paths, contract, service) = synthetic(&temp);
        let first = start_monitor_with_contract(paths.clone(), true, contract.clone()).await.unwrap();
        let candidate = sha(&fs::read(&service).unwrap());
        let control = fs::read(paths.state_root.join("control.json")).unwrap();
        let status = fs::read(paths.state_root.join("status.json")).unwrap();
        assert!(start_monitor_with_contract(paths.clone(), false, contract.clone()).await.is_err());
        assert_eq!(sha(&fs::read(&service).unwrap()), candidate);
        assert_eq!(fs::read(paths.state_root.join("control.json")).unwrap(), control);
        assert_eq!(fs::read(paths.state_root.join("status.json")).unwrap(), status);
        first.stop().await;
        let next = start_monitor_with_contract(paths.clone(), true, contract).await.unwrap();
        assert_eq!(sha(&fs::read(&service).unwrap()), candidate);
        next.stop().await;
    }

    #[tokio::test]
    async fn monitor_shutdown_preserves_external_edits_and_reports_recovery_failure() {
        let temp = tempfile::tempdir().unwrap();
        let (paths, contract, service) = synthetic(&temp);
        let monitor = start_monitor_with_contract(paths.clone(), true, contract).await.unwrap();
        fs::write(&service, b"external edit").unwrap();
        monitor.stop().await;
        assert_eq!(fs::read(&service).unwrap(), b"external edit");
        let control: Value =
            serde_json::from_slice(&fs::read(paths.state_root.join("control.json")).unwrap()).unwrap();
        assert_eq!(control["requireIdentification"], false);
        let status: BrowserStatus =
            serde_json::from_slice(&fs::read(paths.state_root.join("status.json")).unwrap()).unwrap();
        assert_eq!(status.state, "blocked");
        assert!(status.detail.contains("External runtime change"));
        assert_eq!(
            wait_for_monitor_shutdown_at(&paths, Duration::ZERO).unwrap(),
            NativeBrowserShutdown::RestoreFailed
        );
        assert!(acquire_monitor_owner(&paths).is_ok());
    }

    #[tokio::test]
    async fn dropped_monitor_sender_still_runs_recovery() {
        let temp = tempfile::tempdir().unwrap();
        let (paths, contract, service) = synthetic(&temp);
        let original = sha(&fs::read(&service).unwrap());
        let BrowserMonitor { shutdown, task } =
            start_monitor_with_contract(paths.clone(), true, contract).await.unwrap();
        drop(shutdown);
        task.await.unwrap();
        assert_eq!(sha(&fs::read(&service).unwrap()), original);
        assert!(acquire_monitor_owner(&paths).is_ok());
    }

    #[test]
    fn manager_wait_is_read_only_and_refuses_an_active_monitor() {
        let temp = tempfile::tempdir().unwrap();
        let paths = paths(&temp);
        wait_for_monitor_shutdown_at(&paths, Duration::ZERO).unwrap();
        assert!(!paths.state_root.exists());
        let mut owner = acquire_monitor_owner(&paths).unwrap();
        write_monitor_receipt(&mut owner, &uuid::Uuid::new_v4().to_string(), "restored").unwrap();
        assert!(wait_for_monitor_shutdown_at(&paths, Duration::ZERO).is_err());
        assert!(!paths.state_root.join("control.json").exists());
        drop(owner);
        wait_for_monitor_shutdown_at(&paths, Duration::ZERO).unwrap();
    }

    #[test]
    fn manager_rejects_incomplete_receipts_and_legacy_enabled_state() {
        let temp = tempfile::tempdir().unwrap();
        let paths = paths(&temp);
        fs::create_dir_all(&paths.state_root).unwrap();
        fs::write(paths.state_root.join("control.json"),
            br#"{"schema":1,"requireIdentification":true}"#).unwrap();
        assert!(wait_for_monitor_shutdown_at(&paths, Duration::ZERO).is_err());
        let mut owner = acquire_monitor_owner(&paths).unwrap();
        let generation = uuid::Uuid::new_v4().to_string();
        for state in ["active", "blocked"] {
            write_monitor_receipt(&mut owner, &generation, state).unwrap();
            FileExt::unlock(&owner).unwrap();
            let shutdown = wait_for_monitor_shutdown_at(&paths, Duration::ZERO);
            if state == "blocked" {
                assert_eq!(shutdown.unwrap(), NativeBrowserShutdown::RestoreFailed);
            } else {
                assert!(shutdown.is_err());
            }
            owner.try_lock_exclusive().unwrap();
        }
        owner.set_len(0).unwrap();
        drop(owner);
        assert!(wait_for_monitor_shutdown_at(&paths, Duration::ZERO).is_err());
    }

    #[tokio::test]
    async fn manager_wait_finishes_only_after_original_is_restored() {
        let temp = tempfile::tempdir().unwrap();
        let (paths, contract, service) = synthetic(&temp);
        let original = sha(&fs::read(&service).unwrap());
        let monitor = start_monitor_with_contract(paths.clone(), true, contract.clone()).await.unwrap();
        let wait_paths = paths.clone();
        let waiter = tokio::task::spawn_blocking(move || {
            wait_for_monitor_shutdown_at(&wait_paths, Duration::from_secs(5)).unwrap();
            assert_eq!(sha(&fs::read(service).unwrap()), original);
        });
        monitor.stop().await;
        waiter.await.unwrap();
    }

    #[test]
    fn unverified_runtime_is_not_reported_as_file_conflict_or_browser_failure() {
        let temp = tempfile::tempdir().unwrap();
        let (paths, mut contract, service) = synthetic(&temp);
        let before = fs::read(&service).unwrap();
        contract
            .files
            .push(("bin/node.exe", FileCheck::Known("unknown-version".into())));
        // The synthetic descriptor determines the runtime, not the actual user installation.
        let key = discover(&paths).unwrap().unwrap();
        let node = paths.runtime_root.join(key).join("bin/node.exe");
        fs::create_dir_all(node.parent().unwrap()).unwrap();
        fs::write(node, b"new official version").unwrap();
        let error = reconcile_contract(&paths, true, &contract).unwrap_err();
        assert_eq!(error_status(&error).state, "runtime_unverified");
        assert_eq!(fs::read(&service).unwrap(), before);
    }

    // Only temporary Node runs our inspector; the supplied service and worker are not executed.
    #[test]
    #[ignore = "requires CPP_NATIVE_BROWSER_STRUCTURAL_FIXTURE; runs only our parser on a temp copy"]
    fn structural_fixture_transaction_recovery_and_component_drift() {
        let fixture = PathBuf::from(std::env::var_os("CPP_NATIVE_BROWSER_STRUCTURAL_FIXTURE").unwrap());
        let temp = tempfile::tempdir().unwrap();
        let paths = paths(&temp);
        let key = "0123456789abcdef";
        let runtime = paths.runtime_root.join(key);
        for file in [
            SERVICE, "manifest.json", "bin/node.exe", "bin/node_repl.exe",
            "bin/node_modules/@oai/cua-repl/bin/cua-repl.mjs",
            "bin/node_modules/@oai/browser-desktop/package.json",
        ] {
            let target = runtime.join(file);
            fs::create_dir_all(target.parent().unwrap()).unwrap();
            fs::copy(fixture.join(file), target).unwrap();
        }
        let dir = paths.codex_home.join("plugins/cache/openai-bundled/unified-computer-use/test");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join(".mcp.json"), serde_json::to_vec(&descriptor(&paths.runtime_root, key)).unwrap()).unwrap();
        let service = runtime.join(SERVICE);
        let original = fs::read(&service).unwrap();
        let modified = fs::metadata(&service).unwrap().modified().unwrap();
        assert_eq!(reconcile(&paths, true).unwrap().state, "prepared");
        let candidate = fs::read(&service).unwrap();
        assert_ne!(candidate, original);
        assert_eq!(reconcile(&paths, true).unwrap().state, "prepared");
        assert_eq!(fs::read(&service).unwrap(), candidate);
        assert_eq!(reconcile(&paths, false).unwrap().state, "restored");
        assert_eq!(fs::read(&service).unwrap(), original);
        assert_eq!(fs::metadata(&service).unwrap().modified().unwrap(), modified);
        // Parser-qualified snapshots are still exact transaction guards.
        let contract = structural::detect(&paths, key).unwrap();
        fs::write(runtime.join("bin/node_repl.exe"), b"external worker edit").unwrap();
        assert!(prepare(&paths, key, &contract).is_err());
        assert_eq!(fs::read(&service).unwrap(), original);
        // Recovery does not need an executable or an intact generated descriptor.
        fs::copy(fixture.join("bin/node_repl.exe"), runtime.join("bin/node_repl.exe")).unwrap();
        reconcile(&paths, true).unwrap();
        fs::remove_file(runtime.join("bin/node.exe")).unwrap();
        fs::remove_file(dir.join(".mcp.json")).unwrap();
        assert_eq!(reconcile(&paths, false).unwrap().state, "restored");
        assert_eq!(fs::read(&service).unwrap(), original);
    }
    #[test]
    fn stale_cleanup_receipt_is_checked_against_disk_without_writes() {
        let temp = tempfile::tempdir().unwrap();
        let (paths, contract, service) = synthetic(&temp);
        reconcile_contract(&paths, true, &contract).unwrap();
        reconcile_contract(&paths, false, &contract).unwrap();
        let original = fs::read(&service).unwrap();
        let control = fs::read(paths.state_root.join("control.json")).unwrap();
        let mut owner = acquire_monitor_owner(&paths).unwrap();
        let generation = uuid::Uuid::new_v4().to_string();
        for state in ["active", "blocked", "restored"] {
            write_monitor_receipt(&mut owner, &generation, state).unwrap();
            FileExt::unlock(&owner).unwrap();
            assert_eq!(
                wait_for_monitor_shutdown_at(&paths, Duration::ZERO).unwrap(),
                NativeBrowserShutdown::Ready
            );
            owner.try_lock_exclusive().unwrap();
        }
        write_monitor_receipt(&mut owner, &generation, "restored").unwrap();
        drop(owner);
        assert_eq!(fs::read(&service).unwrap(), original);
        assert_eq!(fs::read(paths.state_root.join("control.json")).unwrap(), control);

        fs::write(&service, b"external edit").unwrap();
        for state in ["active", "restored"] {
            let mut owner = acquire_monitor_owner(&paths).unwrap();
            write_monitor_receipt(&mut owner, &generation, state).unwrap();
            drop(owner);
            assert!(wait_for_monitor_shutdown_at(&paths, Duration::ZERO).is_err());
            assert_eq!(fs::read(&service).unwrap(), b"external edit");
            assert_eq!(fs::read(paths.state_root.join("control.json")).unwrap(), control);
        }
    }

    #[test]
    fn blocked_cleanup_preserves_restore_failed_and_external_edits() {
        let temp = tempfile::tempdir().unwrap();
        let (paths, contract, service) = synthetic(&temp);
        reconcile_contract(&paths, true, &contract).unwrap();
        fs::write(&service, b"external edit").unwrap();
        let control = fs::read(paths.state_root.join("control.json")).unwrap();
        let mut owner = acquire_monitor_owner(&paths).unwrap();
        write_monitor_receipt(&mut owner, &uuid::Uuid::new_v4().to_string(), "blocked").unwrap();
        drop(owner);
        assert_eq!(
            wait_for_monitor_shutdown_at(&paths, Duration::ZERO).unwrap(),
            NativeBrowserShutdown::RestoreFailed
        );
        assert_eq!(fs::read(&service).unwrap(), b"external edit");
        assert_eq!(fs::read(paths.state_root.join("control.json")).unwrap(), control);
    }

    #[test]
    fn unknown_cleanup_receipt_is_rejected_even_when_disk_is_restored() {
        let temp = tempfile::tempdir().unwrap();
        let (paths, contract, service) = synthetic(&temp);
        reconcile_contract(&paths, true, &contract).unwrap();
        reconcile_contract(&paths, false, &contract).unwrap();
        let original = fs::read(&service).unwrap();
        let mut owner = acquire_monitor_owner(&paths).unwrap();
        write_monitor_receipt(&mut owner, &uuid::Uuid::new_v4().to_string(), "unknown").unwrap();
        drop(owner);
        assert!(wait_for_monitor_shutdown_at(&paths, Duration::ZERO).is_err());
        assert_eq!(fs::read(&service).unwrap(), original);
    }

    // The proprietary runtime is supplied locally, never committed or executed by this test.
    #[test]
    #[ignore = "requires CPP_NATIVE_BROWSER_FIXTURE and CPP_NATIVE_BROWSER_DESCRIPTOR"]
    fn pinned_fixture_transaction_recovery_and_external_change() {
        let fixture = PathBuf::from(std::env::var_os("CPP_NATIVE_BROWSER_FIXTURE").unwrap());
        let generated = PathBuf::from(std::env::var_os("CPP_NATIVE_BROWSER_DESCRIPTOR").unwrap());
        // 真机夹具可能已经是比登记表更新的版本，此时结构降级契约也必须成立（issue #2294）。
        RuntimeContract::for_manifest(
            &read_regular(&fixture.join(MANIFEST), 1024 * 1024).unwrap(),
            &fixture,
            sha(&read_regular(&fixture.join(SERVICE), MAX_SERVICE).unwrap()),
        )
        .unwrap();
        let mut data: Value = serde_json::from_slice(&fs::read(&generated).unwrap()).unwrap();
        let source_key = selected_key(&data, fixture.parent().unwrap()).unwrap();
        assert_eq!(
            Some(source_key.as_str()),
            fixture.file_name().unwrap().to_str()
        );
        let temp = tempfile::tempdir().unwrap();
        let paths = paths(&temp);
        let key = "0123456789abcdef";
        let runtime = paths.runtime_root.join(key);
        // Preserve Desktop's descriptor spelling; only relocate its checked paths.
        for field in [
            "/mcpServers/cua_repl/command",
            "/mcpServers/cua_repl/env/NODE_REPL_NODE_PATH",
            "/mcpServers/cua_repl/env/CUA_REPL_NODE_REPL_PATH",
            "/mcpServers/cua_repl/args/0",
        ] {
            let value = data.pointer_mut(field).unwrap();
            let relative = Path::new(value.as_str().unwrap())
                .strip_prefix(&fixture)
                .unwrap();
            *value = json!(runtime.join(relative));
        }
        let service = runtime.join(SERVICE);
        fs::create_dir_all(service.parent().unwrap()).unwrap();
        fs::copy(fixture.join(SERVICE), &service).unwrap();
        for file in RUNTIME_FILES {
            let target = runtime.join(file);
            fs::create_dir_all(target.parent().unwrap()).unwrap();
            fs::copy(fixture.join(file), target).unwrap();
        }
        let original = fs::read(&service).unwrap();
        let modified = fs::metadata(&service).unwrap().modified().unwrap();
        let dir = paths
            .codex_home
            .join("plugins/cache/openai-bundled/unified-computer-use/test");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join(".mcp.json"),
            serde_json::to_vec(&data).unwrap(),
        )
        .unwrap();
        assert_eq!(reconcile(&paths, true).unwrap().state, "prepared");
        let candidate = fs::read(&service).unwrap();
        assert_ne!(candidate, original);
        assert_eq!(reconcile(&paths, true).unwrap().state, "prepared");
        assert_eq!(reconcile(&paths, false).unwrap().state, "restored");
        assert_eq!(fs::read(&service).unwrap(), original);
        assert_eq!(
            fs::metadata(&service).unwrap().modified().unwrap(),
            modified
        );
        // Simulate cache rebuild and interrupted deployment with a durable journal.
        assert_eq!(reconcile(&paths, true).unwrap().state, "prepared");
        fs::write(&service, &original).unwrap();
        assert_eq!(reconcile(&paths, true).unwrap().state, "prepared");
        assert_eq!(fs::read(&service).unwrap(), candidate);
        fs::write(&service, b"external edit").unwrap();
        assert!(reconcile(&paths, false).is_err());
        assert_eq!(fs::read(&service).unwrap(), b"external edit");
        fs::write(&service, &candidate).unwrap();
        reconcile(&paths, false).unwrap();
        assert_eq!(fs::read(&service).unwrap(), original);
    }
}


