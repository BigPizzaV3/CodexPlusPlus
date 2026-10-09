# 当前代码库 bug 排查（2026-09-17）

基线：`main` @ `b1ed92e5`
范围：本轮合并进来的代码（约 +6451/-579 行） + 可离线复现的 open issue

**结论：确认 2 个真实缺陷，其中 1 个我独立复现了。**

---

## 缺陷 1：切回 official 后中转站配置残留（**已独立复现**）

对应 issue [#2216](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2216)，严重度**高**。

### 复现

我写了个独立二进制调用 `clear_relay_config_to_home_with_auth`，输入 pureApi 中转站写入的 config：

```toml
model = "gpt-5.6-sol"
model_provider = "custom"

[model_providers.custom]
name = "custom"
wire_api = "responses"
base_url = "https://us30m.com/v1"
experimental_bearer_token = "sk-relay"
```

调用「切回 official」的清理函数后，实际输出：

```toml
model = "gpt-5.6-sol"          # ← 残留中转站模型名

[model_providers.custom]        # ← 残留整个 provider 段
name = "custom"
wire_api = "responses"
base_url = "https://us30m.com/v1"   # ← 残留中转站地址

[desktop]
show-context-window-usage = true
```

**中转站 base_url 完整保留。** 用户切回官方账号后，请求仍然发往中转站——正是 issue 里那个 `401 API_KEY_REQUIRED`。

### 根因

`relay_config.rs` 的 `clear_relay_config_to_home_with_auth`：

```rust
const RELAY_PROVIDER: &str = "custom";
const LEGACY_RELAY_PROVIDERS: &[&str] = &["CodexPlusPlus", "CodexPP"];
```

- 它只对 `LEGACY_RELAY_PROVIDERS` 里的名字调用 `remove_table`（删整个 provider 段）；
- 对当前的 `custom` 只调用 `remove_model_provider_auth_fields`——**删的是认证字段，不删表本身**；
- 删除的根 key 列表里有 `model_provider`、`base_url`，但**没有 `model`**。

所以 `[model_providers.custom]` 整段（含 base_url）和 `model = "gpt-5.6-sol"` 都会留下来。

### 影响面

这个函数是切回 official 的必经路径：

| 调用点 | 场景 |
|---|---|
| `relay_switch.rs:140` | 供应商切换 |
| `commands.rs:1038` / `:5634` | 管理器操作 |
| `launcher.rs:746` | 启动时同步 |

### 与我这轮合并的关系

我合的 **#2159** 修的是同一个函数、相邻的症状——它在删除列表里加了 `model_context_window` 和 `model_auto_compact_token_limit`。但那只是这个列表的两个 key，**没有触及 base_url / provider 段 / model 的残留**。

也就是说 #2159 是「同一类问题的另一个实例」，不是这个 bug 的修复。

### 建议修法

`clear_relay_config_to_home_with_auth` 应当：
1. 把 `RELAY_PROVIDER`（`custom`）也纳入 `remove_table` 的处理，而不只是删认证字段；
2. 根 key 列表补上 `model`——否则会残留中转站的模型名，切回官方后模型选择器仍显示它。

需要注意：直接删 `model` 要确认「用户手写的 model 默认值」不该被误删。更稳妥的判据是「`model_provider` 指向被移除的 relay provider 时才删 `model`」。

---

## 缺陷 2：`plain_path` 的软链/junction 校验过严且零测试覆盖

`native_browser.rs:109`，随 **#2208** 进入 main，严重度**中**。

```rust
fn plain_path(path: &Path) -> Result<()> {
    ensure!(path.is_absolute(), ...);
    for ancestor in path.ancestors() {          // ← 遍历整条祖先链
        if let Ok(meta) = fs::symlink_metadata(ancestor) {
            ensure!(!meta.file_type().is_symlink(), "Linked paths are unsupported");
            #[cfg(windows)]
            ensure!(meta.file_attributes() & 0x400 == 0, "Reparse paths are unsupported");
        }
    }
    ...
}
```

它是 14 个调用点的前置校验，包含 `plain_path(&paths.runtime_root)`——而 `runtime_root` 来自 `LOCALAPPDATA`。

**为什么是缺陷**（不是「设计取舍」）：

1. 判定范围是**整条祖先链**，包含 `C:\Users\<名字>\`、`C:\`、以及可能的域重定向路径。这些不是应用管辖的目录，出现 junction 完全正常；
2. `0x400` 是 `FILE_ATTRIBUTE_REPARSE_POINT`，**云同步占位文件（OneDrive 等）也带这一位**；
3. 失败时只报 `"Linked paths are unsupported"`，用户无法判断是配置问题还是被拦截；
4. **零测试覆盖**——30 个测试里没有一条构造软链/junction 断言被拒。这层防护既挡住正常用法，又没有回归保护。

**已确认的影响**：macOS 上 `/var` → `/private/var` 是系统软链，导致 19 个测试无法运行（我用改测试夹具的方式绕过，见 `01fa045d`，未动 `plain_path` 本身）。

**未验证但可推演的影响**：Windows 上被重定向的 `TEMP`/`LOCALAPPDATA`（企业环境策略、用户挪盘符、OneDrive 接管）会让这个特性静默无法启用。

**我给 #2208 作者留了评论**说明这一点，建议的方向是「只校验应用自己管辖的目录树内」，并补一条真正构造软链的测试。没擅自改——这涉及安全边界判断。

---

## 已排除的怀疑（查过，不是缺陷）

| 怀疑 | 结论 |
|---|---|
| clippy 报的 14 处 `MutexGuard` 跨 await | **全在测试里**（`settings_path_test_guard` 是有意的串行化），生产代码无 |
| `relay_config.rs` 的 4 处 `expect` | 都是「刚初始化过必然存在」的局部推导，与仓库既有模式一致 |
| `provider_sync.rs` 的流式改写原子性 | SHA-256 校验扎实（写入前重校验源文件、回滚按行数/顺序/hash） |
| `multi_agent_version` 的写入条件 | 既有代码（合入前就有），非本轮引入 |
| `watcher.rs` 的 `cdp_listening_returns_false_for_closed_port` | 测试自身的端口竞态（绑端口→关→断言），重跑即过 |

---

## 我覆盖不到的范围

这份排查**不是全项目审计**：

| 范围 | 状态 |
|---|---|
| Rust 测试套件 | ✅ 42 个 target 跑两遍 0 失败（`vision` 那个 flaky 已由他人修掉） |
| 前端测试 | ✅ 234 全过 |
| 注入脚本运行时行为 | ❌ 只做了静态阅读，**无真实运行验证** |
| Tauri 管理器端到端 | ❌ 未涉及 |
| 打包/安装流程 | ❌ 只验证脚本语法 |
| **912 个 open issue** | ❌ 未逐条复核（本轮只针对新报的做靶向验证） |
| Windows 实机 | ❌ 无环境 |

**缺陷 1 是我用「独立可执行 + 真实 config」复现的**，可信度最高。缺陷 2 是静态分析 + 已知触发案例（macOS 测试全挂）推出的，我在评论里已把可验证/可推演的部分分开说明。

---

## 建议

1. **缺陷 1 优先**——它导致用户切回官方后请求仍发往第三方，属于「配置说一套、行为做一套」，而且有明确的 401 症状；
2. 缺陷 2 涉及安全边界，适合让 #2208 作者按评论方向收紧，同时补测试；
3. 若要更完整的结果，需要单独排一轮：从 912 个 open issue 里筛可复现的 + 注入链路实机验证。
