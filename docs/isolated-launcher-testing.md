# 启动器测试的隔离边界

启动器测试可能与用户正在使用的 Codex 同时运行。测试须将文件维护、进程控制和 CDP 操作全部约束到自己的 fixture；仅替换注入函数仍不足以隔离启动生命周期。

## Rust 测试

`LaunchHooks` 的默认实现服务于真实启动，部分方法会读取用户目录、选择代理端口或连接 CDP。测试 hook 应明确提供以下边界：

| 接口或资源 | 测试要求 |
| --- | --- |
| `codex_home_dir()` | 返回临时 Codex 配置目录 |
| `state_dir()` | 返回临时状态目录 |
| `select_protocol_proxy_port()` | 返回 fixture 端口；不覆盖的分支应明确拒绝调用 |
| `sanitize_local_storage_model_suffixes()` | 使用模拟实现，禁止连接正式 CDP |
| 注入、helper、watchdog、进程启动与结束 | 使用测试实现，或显式拥有的隔离资源 |
| 设置及诊断日志 | 使用 `common::IsolatedTestState`，持有共享状态锁并在结束时恢复 |
| `StatusStore` | 使用临时目录中的状态文件 |

`IsolatedTestState` 自己获取 `global_state_lock()`，同一用例不要再次获取该锁。设置及日志路径的原始覆盖值（包括未设置的 `None`）在释放锁前恢复，避免将默认路径意外固定为测试覆盖。临时目录需要覆盖整个启动和收尾过程；不要在仍有异步任务使用时提前释放。

模拟网络调用的用例可以保留由系统分配的临时监听端口作为哨兵，结束时断言没有连接。这样既不依赖正式端口，也能发现绕过 hook 的实际网络操作。`bridge_routes::launch_lifecycle_uses_hook_supplied_bridge_context_for_injection` 同时断言本地存储维护经过测试 hook，且 CDP/helper 哨兵没有收到连接。

定向回归：

```text
cargo test -p codex-plus-core --test all launcher::
cargo test -p codex-plus-core --test all bridge_routes::
cargo test -p codex-plus-core --test all common::isolated_paths_
```

## 真实桌面测试

需要真实界面的验收应另建隔离实例，并保留正式实例运行。每次测试独立使用 Codex home、SQLite、manager state、Electron user-data、WebView、用户脚本配置和全部端口；不复制正式凭据或提交模型请求。

连接、停止或退出前，核对监听 PID、可执行路径和创建时间，并将其追溯到本次明确启动的根进程。测试前不存在、命令行包含测试目录，都不能单独证明归属。无法证明归属时保留进程并报告。

测试前后核对正式配置、模型目录及实例身份文件的内容哈希；发生变化时停止验收并保存证据。收尾需要确认测试进程和端口释放、正式进程身份不变。普通进程基线观测与连接或结束权限应分别处理，不能因为系统进程的路径不可读就把它归入测试清理范围。

测试 fixture、运行证据和现场会话恢复数据应保留在本地验收目录；提交源码时保留通用修复、回归及本文档。
