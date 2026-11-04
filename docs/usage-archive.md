# Daily 消费者封存接口

`/usage/archive` 保存固定 Daily schema 的消费者自有数据。它不代表原生账单、独立计量或供应商归属证明，也不导入、合并或修改 `/usage/history`。`providerSource` 等字段原样作为消费者元数据保存，不提升其可信等级。

接口沿用默认关闭的 `usageHistoryEnabled` 开关。关闭或读取设置失败时返回 `{"status":"unavailable","enabled":false}`，不调用数据服务。home 与 settings 路径由宿主提供，请求不能指定路径、数据库、client 或任意存储键。client 固定为 `daily`。

这是通过现有拓展白名单开放的可写消费者接口；固定 client 是存储命名空间限制，
不是调用者身份认证。它不扩展基础查询的只读权限，`/settings/set`、删除等路由仍不开放。

## 请求与响应

所有请求和 typed 对象拒绝未声明字段。

| 操作 | 请求 | 成功响应 |
| --- | --- | --- |
| list | `{"operation":"list"}` | `{"status":"ok","entries":[meta]}`，按日期升序 |
| read | `{"operation":"read","date":"YYYY-MM-DD"}` | `{"status":"ok","entry":meta+detail}` |
| put | `{"operation":"put","expectedRevision":null,"entry":entry}` | `{"status":"ok","entry":meta+detail}` |

`meta` 是 `{date,revision,core,detailAvailable,protectedDetail}`；`read` 和 `put` 的 `entry` 另有 `detail:string|null`。`list` 不读取或返回大明细。不存在数据库或日期返回 `status:"unavailable"`，不会伪装空的成功列表。

现有 `codexPlus.call` 会把 `status:"failed"` 转成 Promise rejection，并丢弃响应中的固定 `code`；`unavailable` 和 `conflict` 则原样返回。消费者必须同时处理返回状态及 catch，失败时保留本地数据，不依赖 Error 中存在 code。该接口不修改通用桥接的错误包装。

写入 entry 固定为 `{version:1,date,core,detail:string,protectedDetail:boolean}`。put 可另传与 `entry.date` 相同的顶层 `date`。初次写入的 `expectedRevision` 为 null 或省略；更新传先前得到的 revision。CAS 冲突返回 `{"status":"conflict","date":"YYYY-MM-DD","revision":"当前revision或null"}`，不覆盖或触发清理。

revision 为规范化完整提交 entry 的 SHA-256 小写十六进制值：对象键递归排序，数组保留顺序，声明的可选字段省略/null 统一为省略。相同规范 entry 重试成功，即使携带的是提交前的 expectedRevision。不同内容必须匹配当前 revision。只有 SQLite 事务提交成功才能返回 ok。

revision 始终按消费者提交的原始 detail 字符串计算，不按落盘压缩字节计算。read/put 仍返回同一原始字符串，消费者不需要识别服务端的存储压缩格式。

明细过期不会改变 revision，revision 标识最后一次接受的完整提交，不是读取结果的重新哈希。重复原提交不会重新恢复已到期的明细。put 返回实际提交后的 entry，客户端必须看 `detailAvailable`，不能假定旧日期提交后明细仍存在。

## Typed Core

core 是 `{all:Summary,sessions:[{sessionId,summary:Summary}],sessionInfo:{[id]:{role,parentSessionId?}},aliases:{[alias]:canonical},pendingLiveEvents,sourceAccountingFingerprint?}`。

Summary 的必填字段：

```text
input, output, cached, reasoning, total, calls, turns
cachedKnown, reasoningKnown, inputKnown, outputKnown, totalKnown, usageUncertain
models: ModelRow[]
breakdowns: BreakdownRow[]
```

两种 row 同样必填上述七项计数和六项布尔字段，但没有 models/breakdowns 子数组。ModelRow 另含必填 `model`，可选 `modelInferred,inferredModelTokens`。BreakdownRow 另含必填 `model,provider,role`，可选 `providerName,modelConflict,providerConflict,providerInferred,modelInferred,inferredTokens,inferredModelTokens,providerSource,modelSource`。

SessionInfo 和 BreakdownRow 的 role 只允许 `main`、`subagent`、`unknown`。空串、大小写变体及其他值均拒绝，后台不静默归一化；需要兼容旧空角色的消费者应在构建 core 时显式归一化。

每个计数为非负整数且不超过 JS safe integer。每一行都满足 `total=input+output`、`cached<=input`、`reasoning<=output`，推断 token 数不超过对应行 total。每份 Summary 的 models 总和与 breakdowns 总和均等于该 Summary 的七项计数；sessions 的 Summary 总和等于 all，包含 calls/turns。

sessionId、sessionInfo 的键、父会话及 aliases 两端必须是小写规范 UUID，不接受 `local:`、路径或非规范拼写。sessions 不得重复且必须有 sessionInfo；父子关系不得自指或成环。aliases 指向最终规范 ID，不接受多级别名链或环。允许父会话不在当天有消费，且不根据角色自动推断父会话。

所有短字符串最多 256 个 Unicode 字符。sessions、sessionInfo 和 aliases 各最多 4096 项。core 紧凑 JSON 最多 512 KiB。detail 是 UTF-8 最多 8 MiB 的 opaque 字符串，服务端只处理自己的存储压缩层，不解压消费者内层格式、不解析、执行或记录其内容。它不能用作计量或来源证明。pendingLiveEvents 大于零时必须 protectedDetail=true；其他未决情况由消费者主动置保护标记。

## 存储与保留

独立的 `usage-summary.sqlite` 位于与现有供应商证据相同的 home 分桶目录。它有独立 schema/version，并绑定规范化的 home 与 state 目录；把数据库复制到另一 home/state 后会被拒绝，不重写 owner。原有 `usage-provider-cache.sqlite` 不受此接口影响。

schema 1 的新 detail 列声明为 BLOB。新内容使用已有 zstd 依赖按 level 1 压缩 UTF-8 字节并带帧校验和，不使用 base64；Daily 的 dense16 字符串不会直接按 UTF-8 TEXT 落盘。读取按 SQLite 实际值类型区分：BLOB 为服务端 zstd 帧，TEXT 为旧版原字符串，NULL 为无明细。既有 schema 1 的 TEXT 列及其中的 TEXT 值仍可读取；正常 CAS 更新写为 BLOB，无需改表或迁移。相同 entry 的幂等重试不改写既有 TEXT。

解压采用流式 decoder，窗口最多 8 MiB，输出只读取 `MAX_DETAIL_BYTES + 1` 字节以检测超限。无论压缩帧声明什么原始长度，超出 8 MiB、损坏/截断帧、校验和错误或非 UTF-8 内容都返回 failed，不产生部分成功。压缩 BLOB 本身最多 8 MiB 加 64 KiB，以容纳不可压缩输入的封装开销。

list/read 使用只读连接，不创建数据库、目录、迁移、采集器或索引，不执行过期清理。新数据库使用回滚日志模式，无需创建 WAL/SHM。数据库损坏、未知 schema、owner 不符及 IO/SQL 错误返回 failed；响应只带固定 code，不回显内容或本机路径。

core 永久保留。每次被接受的 put 在同一事务内清空超过 30 个本地日且未保护的 detail，即 `date < local_today - 30 days`；恰好第 30 天仍保留。protectedDetail=true 不自动淘汰。所有现存 Daily detail 还原后的 UTF-8 字节合计最多 128 MiB，先按上述规则修剪，再检查限额；压缩不会扩大逻辑配额。配额检查逐条有界解压计数，最多同时还原一日明细，不依赖可伪造的压缩帧长度或额外长度缓存。超限或保留明细解码失败时，新写入及修剪一起回滚，旧数据与保护明细全部保留。读取不会刷新或改变保留期；list 不读取或解压 detail。

## 消费者边界

消费者应确认按会话及分类精确分配、保存父子信息后再封存；不能把无法拆分的 backendSummary 随意归给某个 session。本服务只验证固定结构及七项计数守恒，不逐分类比对 sessions 与 all 的 model/provider/role 分布，不能证明源数据完整或分类正确。完整分类守恒由消费者在封存前后核验。

archive 与 raw history 是互斥的读取来源，不应相加。sealed 日期需要避开 history 的摘要替换、全局游标重解释及缺失日期清零路径。sourceAccountingFingerprint 只是原样保存的消费者来源标识；迟到来源变化应提示待复核，不自动覆盖已封存 core，也不能由历史缺失推导出零消费。

测试入口为 data 的 `--lib usage_archive::tests::` 以及 core 的 `--test all usage_history_routes::archive_`。运行时仍须使用项目要求的隔离 home/state/SQLite/scripts/temp 环境，避免与其他 Cargo 作业并行。

数据层包含真实临时 SQLite 文件的压缩回归：大尺寸 dense-like Unicode 精确往返、BLOB 存储类别/压缩字节及文件体积收益、旧 TEXT 只读兼容和 CAS 更新、损坏/超限帧与非法 UTF-8 拒绝、8 MiB 输入边界和解压字节配额回滚。压缩收益断言使用合成高熵 Unicode，不代表已测量真实 Daily 数据的压缩率。
