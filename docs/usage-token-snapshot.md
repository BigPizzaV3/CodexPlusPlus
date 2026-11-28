# Token 快照存储基线

这一步只验证独立 SQLite 的 Token 断点、累计基线和观察记录事务。存储模块只在测试中启用，不切换 `/usage/history`、不创建生产数据库，也不存储 Skill、MCP 或普通工具调用。未知的 `user_version` 在任何写操作前拒绝打开；未来正式接入时由调用方保留原内存读取路径。

## 复现

在仓库根目录执行。生成器只写入一个此前不存在的目录；以下数据不依赖真实会话或上游模型。

```powershell
node scripts/generate-token-snapshot-fixture.mjs output/token-snapshot-fixture 30000
$env:CODEX_USAGE_TOKEN_FIXTURE = (Resolve-Path output/token-snapshot-fixture).Path
cargo test -p codex-plus-data --lib synthetic_token_snapshot_baseline -- --ignored --nocapture
cargo test -p codex-plus-data --lib token_checkpoint::tests::
```

每个合成会话有一条用量记录：输入 10（缓存 4）、输出 2、总量 12。30,000 个会话重开后应读回输入 300,000、缓存 120,000、输出 60,000、总量 360,000。基线报告逐文件扫描写入、重开读回的耗时和 SQLite 主文件体积，供比较；这些数字不是容量上限或正式查询延迟承诺。
