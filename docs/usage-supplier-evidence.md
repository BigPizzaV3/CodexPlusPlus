# 独立的离线供应商补证

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
