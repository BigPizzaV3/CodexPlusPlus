# CodeX 插件市场

管理工具的「CodeX 插件市场」可以检索、筛选并按需下载安装插件。Codex 侧复用原生插件页面，注入层拦截插件请求并接入相同后端。

## 下载行为

浏览市场只获取 `plugin-index.json`，不克隆整个插件仓库，也不检出一个包含千个插件的市场目录。点击安装时，后端读取索引中的固定 Git revision，用 `blob:none` 和精确的单目录检出获取所选插件，核对 Git tree 后写入 Codex 插件缓存。实际网络量还包括 Git 的树元数据；需要本机可运行 Git。

公开索引为 520 个条目（约 283 KiB）；当前完整索引为 5,491 个条目（约 4.4 MiB）。完整缓存仓库保持私有，使用已有 `gh auth login` 登录信息访问。GitHub 凭据只用于本次请求，不写入索引、插件包或日志。完整库可访问时，原生列表优先显示完整库；不可访问时显示公开库并保留加载错误。

## 原生接口适配

注入层支持直接 `sendRequest`、原型客户端、旧 `fetch` 消息和 `mcp-request` 消息。接管方法包括：

- `plugin/list` / `list-plugins`：在原生列表中加入我们的索引市场，保留内置插件与其他市场。
- `plugin/read` / `read-plugin`：仅接管带有 CodeX 索引市场标记的插件详情；尚未安装时不虚构本地 Skills 路径。
- `plugin/install` / `install-plugin`：仅接管我们的索引条目，调用单包后端；不再次调用原生安装器。
- `plugin/installed`：从本地安装记录提供已安装条目，不为聊天初始化下载完整索引。

虚拟市场的路径使用 Codex home 内的绝对哨兵路径，实际安装由每个来源和插件 ID 的独立市场标识管理。同名插件的变体不会互相覆盖。远程主机请求不安装到本机。开关沿用「插件市场解锁」；关闭后停止接管。

安装请求只提交一次。桥接超时后查询原任务，失败或无法确认时显示错误，不回退到原生安装器，也不自动重试安装。安装后新开聊天或重启 Codex，使插件被重新加载。连接器和 MCP 服务的账号授权仍由相应服务决定，下载插件文件不会创建服务权限。

## 代码与维护

- `crates/codex-plus-core/src/plugin_market.rs`：索引、分包下载、安装和原生协议适配。
- `assets/inject/renderer-inject/62-plugin-market-adapter.js`：原生页面请求拦截与响应合并。
- `apps/codex-plus-manager/src/PluginMarketScreen.tsx`：管理页面。
- `scripts/build-plugin-market-index.mjs`：从缓存仓库的已提交文件生成轻量索引。

索引生成示例：

```sh
node scripts/build-plugin-market-index.mjs /path/to/CodexPlusPlusPluginCache public /tmp/public-index.json
node scripts/build-plugin-market-index.mjs /path/to/CodexPlusPlusFullPluginCache full /tmp/full-index.json
```

将输出作为相应缓存仓库根目录的 `plugin-index.json` 提交。索引 revision 固定到插件内容所在的已有提交，发布索引不会改变包的校验值。索引生成只读取 Git 已提交内容，忽略本机未提交的文件。

## 验证

```sh
cargo test -p codex-plus-core plugin_market::tests --lib
node --test assets/inject/plugin-market-adapter.test.cjs
node --test apps/codex-plus-manager/src/plugin-market-model.test.ts
node scripts/assemble-renderer-inject.mjs --check
```

显式联网测试在临时 Codex home 中安装两个样本，不修改用户的 Codex 配置，也不调用原生授权流程：

```sh
cargo test -p codex-plus-core --test plugin_market_live -- --ignored --nocapture
```

2026-10-08 验证：公开和完整索引读取成功，分别安装 Google Drive 和 Life Sciences Databases；原生 `skills/list` 加载 49 个 Skills、0 个错误。原生客户端、两类消息协议、超时恢复、远程主机隔离和插件身份去重复均有行为测试。当前运行的旧版本需要重新构建管理工具和启动器后重启，才能加载新增后端和注入代码。
