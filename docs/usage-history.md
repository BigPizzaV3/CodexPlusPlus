# 本机结构化用量接口

`/usage/history` 默认关闭，需开启 `usageHistoryEnabled`。首次查询才创建索引，
没有查询约一分钟后释放；关闭设置等待正在进行的扫描结束。请求不能指定文件路径。
原始会话、配置和登录信息均不修改，不发起模型请求。

## 查询与响应

支持 `period`、`from`、`to`、`model`、`sessionId`、`turnId`、`search`。
`startAt` / `endAt` 为成对的毫秒时间戳，使用 `[startAt,endAt)` 半开区间。
`scopeSessionId` 仅统计明确的子智能体后代，普通 fork 不算子智能体。
`includeSessionCursors` 和 `includeActivity` 必须显式开启；
`activityPage` 每页至多 512 条，`activityRevision` 可避免重复传输未变活动。

响应为 `accountingVersion:2` 的结构化 `days`，包含模型、供应商和
`main/subagent/unknown` 分类、会话别名及父子关系、完整性和累计水位。
本层替换基础读取器响应，不提供旧版的费用、价格和挂件展示字段；
上游鲸鱼挂件仍使用独立的读取器、设置和路由。
调用方必须按版本消费，不能把累计水位当作消费增量。

同一会话所有分段参与汇总；共享观测去重，独立计数流分别计量。
缺少基线、计数冲突和损坏时间保持 `partial`，不把历史累计计入当前日期。
缓存输入是输入的子集，总量为输入加输出；`cachedKnown` /
`reasoningKnown` 为 false 时不能把缺失计数当作精确的零。
`accountingFingerprint` 和 `membershipFingerprint` 绑定事件与计数，
不能单独证明扫描完整或分类正确。

工具活动只保留已完成调用的成功、失败、退出状态及技能分类元数据，
不返回命令、真实路径、正文和完整调用参数。默认不返回活动。

## 资源与生命周期

支持活动和归档目录中的 `.jsonl` / `.jsonl.zst`，最近 365 个本地日期。
文件状态保留预算 64 MiB，每轮输入和解压预算各 4 MiB，单行上限 1 MiB。
预算是索引准入估算，不是整个进程的内存上限。
跳过无关长行，相关或不可辨认的长行保守标记不完整，并继续后续有效记录。
查询驱动扫描，每十秒最多刷新一次；空闲计时器只释放内存，不扫描会话。
`scan.processing=true` 时可能仍返回上一快照，应等待本轮刷新完成。

派生证据与计数冲突保护保存在 Codex++ 设置目录的
`usage-history/<home-id>/usage-provider-cache.sqlite`，不同 Codex home 分开。
原生 SQLite 日志尊重显式的 `CODEX_SQLITE_HOME`，不能退回另一实例目录。
缓存不可用时保守报告完整性，不能把缺失事件解释为零消费。

## 归属证据

事件自身明确归属优先。请求日志、实际 HTTP 完成日志必须按会话、时间、
模型和完整计数核验，双向不唯一、冲突或失败的请求保持未知。
模型命名空间与当前供应商选择都不能证明历史账单归属。
实时通知允许完成日志晚到至多一秒，但请求必须在通知前开始；
完整计数、双向唯一性、顺序、竞争撤销和持久化重验仍保留。
归属修复不会产生新用量。

设置目录既有 `usage-provider-evidence.json` 可提供版本 1 的事件绑定补证，
最多 32 MiB / 100,000 条。事件 ID、会话、时间、角色和计数均需匹配；
精确上游证据可补缺失模型，用户分类不能补模型或进入实时精确证明。
来源通过 `providerSource` / `providerEvidence.sources` 保留。
导入是本地只读消费，不写原始日志；删除文件后下一轮撤销补证。

## 独立的离线供应商补证

设置目录可另放 `usage-supplier-evidence-v2.json`，与版本 1 文件独立，
最多 32 MiB / 100,000 条。所有嵌套对象拒绝未知或缺失字段。
外部核验工具负责原始账单真实性、时间覆盖及所有相关供应商和本地竞争项；
运行时不重新读取全量账单，因此返回
`trustBoundary=external-offline-record-bound`、`runtimeCompetitionRecomputed=false`。

顶层为 `{version:2,records:[...]}`。每条包含：

- `event`：64 位十六进制 `eventId`、规范 UUID `session`、`timestamp`、
  `recordedModel`、`role`、`input/output/cached`，必须与原生事件逐项绑定。
- `supplierEvidence`：`provider/providerName/source/upstream/sessionBinding/competition`，
  来源固定为 `upstream-counter-bidirectional-supplier-only`。
  上游保留真实 supplier、账单 ID、request ID、会话、时间、模型和三计数；
  时间差不超过 30 秒，计数逐项相等。会话来源为直接脱敏字段拷贝，
  不声称请求头身份已认证。
- `competition`：明确 30 秒主窗口、5 分钟保护窗、竞争数量及唯一事件/账单 ID，
  `modelFilter=false`。这些字段是外部核验元数据，不是运行时重验结论。
- `modelEvidence`：`status=conflict/action=preserve-recorded`，
  保留不同的原生模型与真实账单模型，不用改写模型制造匹配。

缓存未知、计数不确定、上下文冲突、重复或改写身份拒绝使用；
事件、账单和 request ID 不能复用。读取前后校验文件和打开句柄身份。
只补通用或缺失供应商，保留原模型、用量和事件身份；`modelRepairs=0`。
已有明确其他供应商的记录不覆盖，版本 2 不进入模型精确证明或 live `verifiedUsage`。
删除文件后下一轮从基础快照重建并撤销补证。

响应独立报告 `supplierEvidence`，保留有限的模型冲突明细，
分组使用 `supplierClassificationEvidence` 标记供应商单维证明；
它不能替代 `classificationEvidence` 的模型精确归属。
