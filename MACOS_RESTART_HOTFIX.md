# Codex++ macOS 重启修复记录

记录时间：2026-05-24

## 背景

问题表现为：点击“重启 Codex”或“启动 Codex++”时，旧 Codex 能被关闭，但新 Codex 有时拉不起来；连续重启时更容易复现。纯 API 模式下插件入口不完整注入，也和这条启动链路有关。

## 根因

macOS 上旧的 Codex 相关进程退出不完整时，可能继续占用这些端口：

```text
9229  调试端口
57321 Codex++ helper / protocol proxy 端口
8732  launcher guard 端口
```

最典型的一次复现里，`SkyComputerUseService` 占用了 `127.0.0.1:9229`。Codex++ 随后虽然带着 `--remote-debugging-port=9229` 启动 Codex，但 CDP `/json` 连接不到目标 Codex，导致插件注入失败。

这个问题不能靠把某一个 sidecar 加进关闭名单解决，因为以后可能是其他 Codex 工具或 sidecar 占端口。正确修法是按端口查占用者，再只关闭命令行属于 Codex、Codex++ 或 `~/.codex` 的进程。

## 修补策略

1. 启动或重启前先解析目标 Codex app 路径。
2. 关闭 Codex++ launcher 旧进程。
3. 关闭目标 Codex app 旧进程。
4. 检查调试端口、helper 端口和 launcher guard 端口。
5. 只关闭这些端口上命令行匹配 Codex/Codex++/`~/.codex` 的进程。
6. 等待端口释放后再启动新 Codex。
7. macOS `open` 命令加入 `-n`，确保新参数不会被复用到旧实例时吞掉。

## 修改位置

```text
apps/codex-plus-manager/src-tauri/src/commands.rs
crates/codex-plus-core/src/watcher.rs
crates/codex-plus-core/src/launcher.rs
crates/codex-plus-core/tests/watcher.rs
crates/codex-plus-core/tests/launcher.rs
```

## 关键函数

启动前清理入口：

```rust
prepare_launch_environment
wait_for_launch_ports_to_clear
```

端口占用者过滤：

```rust
stop_codex_related_processes_listening_on_ports
filter_macos_codex_related_port_owner_processes
```

macOS 新实例启动：

```rust
build_macos_open_command
```

## 验证命令

```bash
cargo test -p codex-plus-core --test launcher --test watcher
```

本次验证重点：

```text
launcher: macOS open 命令包含 -n
watcher: 只关闭 Codex 相关端口占用者，不误杀无关进程
```

## 下次复查

如果以后重启问题再次出现，先查这几件事：

1. `build_macos_open_command` 是否仍包含 `-n`。
2. `prepare_launch_environment` 是否仍会调用 `stop_codex_related_processes_listening_on_ports`。
3. `wait_for_launch_ports_to_clear` 是否仍检查 9229、57321 和 launcher guard 端口。
4. 端口占用者过滤是否仍基于 Codex app 路径、Codex++ bundle 路径和 `~/.codex`，而不是硬编码某个 sidecar 名称。
