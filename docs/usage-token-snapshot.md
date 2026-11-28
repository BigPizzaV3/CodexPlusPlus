# Token 快照存储基线

这一步只验证独立 SQLite 的 Token 断点、累计基线和观察记录事务。存储模块只在测试中启用，不切换 `/usage/history`、不创建生产数据库，也不存储 Skill、MCP 或普通工具调用。只读探测已有数据库，拒绝未知 `user_version` 或带有现有表结构的零版本库，不以读写方式打开这些库。未来正式接入时保留原内存读取路径。

从同一来源的已提交断点继续读，只写该位置之后的新观察。物理位置独立于事件 ID，没有 `ordinal` 的旧日志也不会因为 ID 相同而丢记录。底层读取检测到文件替换或重置后，存储在同一事务中替换该来源的观察和断点；仍被其他来源使用的会话行不受影响。

## 正式接入前确定格式

本阶段的格式只用于复现实验。原事件 ID 与归并使用的 ID 都以 BLOB 存储。JSON 只为可选字段提供默认值，现有计数与覆盖标记仍必须完整；同一版本允许增加不改变既有语义的可选字段。字段重命名保留已有序列化键，改变含义或结构时走显式版本迁移。

第 3 步接入调度和日汇总之前，确定正式格式以及日期、模型、供应商身份、各项 Token 增量（含缓存是否已知）的独立列和索引。日汇总不应逐行解码观察 JSON。保留原事件身份、归类证据与计价覆盖字段，同时在固定截止时间验明新旧路径一致并可回退。

## 复现

在仓库根目录执行。生成器只写入一个此前不存在的目录；以下数据不依赖真实会话或上游模型。

```powershell
node scripts/generate-token-snapshot-fixture.mjs output/token-snapshot-fixture 30000
$env:CODEX_USAGE_TOKEN_FIXTURE = (Resolve-Path output/token-snapshot-fixture).Path
cargo test -p codex-plus-data --lib synthetic_token_snapshot_baseline -- --ignored --nocapture
cargo test -p codex-plus-data --lib token_checkpoint::tests::
```

每个合成会话有一条用量记录：输入 10（缓存 4）、输出 2、总量 12。30,000 个会话重开后应读回输入 300,000、缓存 120,000、输出 60,000、总量 360,000。基线报告逐文件扫描写入、重开读回的耗时，以及 SQLite 主文件和关闭前 WAL 体积。它不含长会话、供应商归类或索引汇总，不能单凭这组数决定生产容量或查询延迟。
