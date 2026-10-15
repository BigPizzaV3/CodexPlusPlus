# 会话分页

管理器的 `list_local_sessions` 在后台线程读取本地会话，接口继续返回
`offset`、`limit`、`hasMore` 和 `totalCount`。

`LocalSessionPager` 缓存会话 ID、更新时间和来源，翻页只读取本页详情。
跨库同 ID 保留较新的记录，时间相同时保留数据库输入顺序。子会话过滤与
`SQLiteStorageAdapter` 共用查询，不保存常驻数据库连接。

缓存检查数据库及 WAL/SHM 的文件信息和头部，最多复用 30 秒。每个数据库
使用短读事务；多个数据库之间没有共同快照，因此外部写入期间翻页结果可能变化。
单库读取失败时返回错误和健康库的结果，不把旧缓存当成成功结果。

回归覆盖深分页、跨库去重、WAL 提交、数据库替换、缓存期限、部分失败及文件句柄释放：

```sh
cargo test -p codex-plus-data storage::session_paging
cargo test -p codex-plus-manager --lib list_local_sessions
```
