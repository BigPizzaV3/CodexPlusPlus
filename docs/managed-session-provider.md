# 历史聚合会话的供应商兼容配置

历史聚合会话可能在 rollout 中保存 `codex_plus_managed`，而当前供应商已经切换。
主进程直接执行原生恢复时不一定经过渲染层补丁，因此配置中需要一个可解析的同名供应商。
兼容表复制当前生效传输，不选择为默认供应商，不保留旧代理地址或凭据。
活动 `model_provider` 无法解析时移除旧兼容表；不会猜测连接目标。

启动与激活已有实例时，仅在供应商接管开启且当前不是网关供应商时修复。
已有兼容表直接更新；缺少时只查 SQLite 索引，不读取 rollout 文件。
通用配置写入和协议代理端口修复只维护已有兼容表，不扫描历史。
只有显式应用或切换供应商后、仍缺少兼容表时，才检查索引引用的 rollout 前八行。
因此旧索引已经改名、仅 rollout 保留内部身份的场景需要重新应用供应商；启动不承担迁移扫描。
没有引入持久缓存；无命中的显式供应商应用可以再次扫描，已有兼容表则跳过。

`prepareCodexManagedSessionConnection` 负责经过渲染层的请求：聚合时注入会话连接快照，
离开聚合后把旧会话恢复请求改为当前供应商。兼容配置覆盖未经过该入口的主进程原生恢复。
`provider_sync` 继续按自己的开关与目标迁移历史索引及其负责的元数据；兼容配置修复只读历史，
不会替代或自动启用供应商同步。

官方登录使用 `requires_openai_auth = true`，不继承第三方凭据；显式官方 HTTP 传输继续保留。
回归测试仅在临时目录使用合成登录内容，解析写出的配置并检查认证文件、历史身份与传输选项。
这些测试不发起真实登录或模型请求，不等同于原生客户端端到端恢复验收。

可选的 `managed_session_provider::native_official_resume_requires_managed_alias_and_preserves_history_after_restart`
使用真实 CLI app-server 和合成历史。设置 `CODEX_PLUS_TEST_CODEX_BIN` 为原生 Codex 可执行文件后运行：

```sh
cargo test --offline -p codex-plus-core --test all managed_session_provider:: -- --ignored --test-threads=1
```

用例先检查缺失 provider 的原生恢复错误，再调用生产配置修复函数，验证恢复后的历史及重启后的按 ID 恢复。
修复检测使用独立的合成 SQLite 索引；app-server 自己使用另一临时 SQLite 目录，由原生程序创建完整数据库。
子进程清空继承环境并隔离 HOME、CODEX_HOME、CODEX_SQLITE_HOME、APPDATA 等目录，凭据存储限定为临时文件。
仅允许 initialize、thread/resume、thread/read 请求，不发起 turn/start；退出和断言失败均清理本次进程树。
该用例验证原生恢复与历史读取，不验证真实官方登录、鉴权或生成。
