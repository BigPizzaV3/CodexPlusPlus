# 修复报告：macOS App bundle 空壳导致管理工具点击无法启动

> 日期：2026-08-17  
> 范围：CodexPlusPlus macOS 安装流程（`crates/codex-plus-core/src/install/macos.rs`）  
> 现象：强杀（force quit）`Codex++ 管理工具` 后，点击图标无法再启动，无窗口、无日志、无任何反馈。

---

## 一、现象与复现

- 用户强杀 `Codex++ 管理工具`（bundle id `com.bigpizzav3.codexplusplus.manager`，菜单栏型应用 `LSUIElement=true`）后，再次点击启动图标没有任何反应。
- 诊断日志 `~/.codex-session-delete/codex-plus.log` 显示 18:51:54 有两个实例（pid 52926/52936）`manager.start` 后立即 `manager.already_running`（guard_port 57319）退出——说明单实例守卫判定"已在运行"，实例静默退出。
- 进一步检查发现 **两个 App bundle 均为空壳**：
  - `/Applications/Codex++ 管理工具.app/Contents/MacOS/CodexPlusPlusManager` = 100 字节 sh 包装脚本（`exec "$DIR/codex-plus-plus-manager" "$@"`），而真正的二进制 `codex-plus-plus-manager` **不存在**；
  - `/Applications/Codex++.app/Contents/MacOS/CodexPlusPlus` = 92 字节包装脚本，二进制 `codex-plus-plus` 同样缺失；
  - `Contents/Resources/` 为空，`Info.plist` 正常（版本 1.2.47）。
- 复现启动：`open -n -b com.bigpizzav3.codexplusplus.manager` 无进程产生；直接执行包装脚本报 `No such file or directory`。
- **根因结论**：bundle 被安装流程重写为"只有包装脚本、没有真实二进制"的空壳（文件 mtime 19:02 为被重写时间），点击时 Launch Services 执行包装脚本，`exec` 不存在的二进制，静默失败。

---

## 二、安装代码缺陷分析

安装入口：`install_entrypoints` / `repair_entrypoints`（`install/mod.rs`）→ `macos::install_app_bundles`（`macos.rs`）→ `write_bundle`。

### 2.1 缺陷一：`write_bundle` 静默跳过二进制拷贝（根因）

修复前（`macos.rs`）：

```rust
if let (Some(source), Some(target_name)) = (&bundle.binary_source, &bundle.binary_target_name) {
    if source.exists() {
        let target = macos.join(target_name);
        if source != &target {
            fs::copy(source, &target)?;   // 仅当 source 存在且 source != target 才拷贝
        }
    }
}
```

两条静默失败路径：

1. **`source.exists()` 为 false** → 拷贝被静默跳过，但流程继续写包装脚本、写 Info.plist，最终返回 `Ok(())`，UI 上报"入口已修复/已安装"。生成的结果就是一个**空壳 bundle**。
2. **`source == &target`**（自修复场景，见 2.2）→ 跳过拷贝但从不校验目标是否真实存在。

### 2.2 缺陷二：自修复 no-op（source == target）

`build_app_bundle` 通过 `option_or_current_exe` + `install_binary_source` 解析二进制来源：

- 管理工具从 bundle 内运行时，`current_exe()` 即 bundle 内路径，`macos_preferred_bundle_binary` 优先返回 sidecar（`codex-plus-plus-manager`），若存在则 binary_source == 安装目标 == bundle 内自身路径 → `source == &target` → 拷贝被跳过。
- 一旦二进制曾因缺陷一丢失，自修复**永远无法恢复**二进制：解析到的 source 是包装脚本自身或缺失路径，且旧代码对这两种情况都不报错。

### 2.3 缺陷三：DMG 布局与安装布局不一致

- DMG 内布局：`MacOS/CodexPlusPlusManager`（37MB 真实二进制），**没有** sidecar 二进制；
- 安装/自修复生成的布局：`MacOS/CodexPlusPlusManager`（100B 包装脚本）+ `MacOS/codex-plus-plus-manager`（真实二进制）。
- `install_binary_source` 只检查 `target.parent()/binary`（sidecar 名），若当时当前可执行文件是 DMG 布局的 `CodexPlusPlusManager`，sidecar 不存在 → 返回 target（真实二进制，能正常拷贝）；但若当前可执行文件是已损坏 bundle 内的包装脚本（100B），则返回的就是**包装脚本本身**作为"二进制源"——拷贝出的目标文件是 sh 脚本，bundle 依旧无法启动，且不报错。

### 2.4 缺陷四：无错误传播、无产物校验

- 二进制缺失/拷贝失败时无任何 `bail!`，`InstallActionResult.status` 仍为 `"ok"`，用户侧完全无感知。
- 写入后不校验 `MacOS/` 下二进制是否存在、是否可执行、是否真是 Mach-O（而非 sh 脚本）。

---

## 三、代码修复（`crates/codex-plus-core/src/install/macos.rs`）

### 3.1 `write_bundle`：缺失即报错，不再静默跳过

```rust
let (source, target_name) = match (&bundle.binary_source, &bundle.binary_target_name) {
    (Some(source), Some(target_name)) => (source, target_name),
    _ => anyhow::bail!("缺少二进制源信息，无法生成可启动的 App bundle（路径：{}）", ...),
};
if !source.exists() {
    anyhow::bail!("二进制源不存在：{}. ...请从 Codex++ DMG 重新安装...");
}
if is_shell_script(source) {
    anyhow::bail!("二进制源是 shell 脚本而非可执行文件：{}. 该 bundle 已损坏，请从 Codex++ DMG 重新安装");
}
let target = macos.join(target_name);
if source != &target {
    fs::copy(source, &target)?;
} else if !is_real_binary(&target) {
    anyhow::bail!("二进制位于 bundle 自身（{}），但目标文件缺失或非可执行文件，无法自修复。请从 Codex++ DMG 重新安装");
}
fs::set_permissions(&target, fs::Permissions::from_mode(0o755))?;
```

关键点：

- `binary_source` / `binary_target_name` 缺失 → 直接报错；
- `source` 不存在 → 报错（提示重新安装）；
- `source` 是 sh 脚本（`#!` 开头）→ 报错（识别"用包装脚本当二进制源"的缺陷三场景）；
- `source == target` 且目标不是真实二进制 → 报错，杜绝自修复静默 no-op；
- 修复后无论如何都强制 0755。

### 3.2 新增校验辅助函数

```rust
fn is_shell_script(path: &Path) -> bool {
    fs::read(path).ok().map(|bytes| bytes.starts_with(b"#!")).unwrap_or(false)
}

fn is_real_binary(path: &Path) -> bool {
    fs::metadata(path).map(|m| m.len() > 1024 && !m.is_dir()).unwrap_or(false)
}
```

### 3.3 `install_binary_source`：增加 companion 兜底

- sidecar 不存在时，若 target 缺失、为 `.` 或为 sh 脚本，尝试以 `current_exe()` 同目录下的 `binary` 文件（非脚本）作为来源，降低自修复时把包装脚本当源的几率。

---

## 四、现场修复（用户机器，无需重装 DMG）

1. 挂载 `~/Downloads/CodexPlusPlus-1.2.47-macos-arm64.dmg`；
2. 从 DMG 拷贝真实二进制到对应 bundle：
   - `Codex++ 管理工具.app/Contents/MacOS/codex-plus-plus-manager`（来自 DMG 的 `CodexPlusPlusManager`，37.5MB，chmod 755）；
   - `Codex++.app/Contents/MacOS/codex-plus-plus`（来自 DMG 的 `CodexPlusPlus`，18MB，chmod 755）；
3. 验证：
   - `open -n -b com.bigpizzav3.codexplusplus.manager` → 进程 `codex-plus-plus-manager` 存活，`lsof -iTCP:57319` 显示 LISTEN，日志新增 `manager.start`；
   - `kill -9` 强杀后再次点击启动 → 新实例正常起来（端口 57319 重新 LISTEN，`manager.start` 出现）。强杀→重启闭环验证通过。

---

## 五、遗留与建议

- 本机无 Rust 工具链（`cargo` 不存在），代码修复**未经编译/测试验证**。建议在 CI 或有工具链环境执行：
  ```bash
  cargo fmt --all -- --check
  cargo test -p codex-plus-core
  cargo build --release
  ```
- 建议后续在 CI 增加 macOS 冒烟：安装入口后校验 `MacOS/` 内二进制存在、非脚本、可执行（对应本报告 2.4）。
- 现场修复仅补回了二进制；若用户之后再次触发安装/修复入口且二进制源解析异常，新代码会**明确报错**而不是静默生成空壳。
