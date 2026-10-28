# Agent 桌宠

在 Codex 里放一只可拖动的桌宠。点击时会压扁再弹回，并播放按压 / 松开音效。

## 开启

打开管理器「Agent 桌宠」页，打开开关即可，默认关闭。目前只支持 Codex，其他 Agent 之后再补。

页面里可以设置：

- **图片**：从内置图库挑一张，或选「自定义」并填一个 `https://` 图片地址；
- **大小**：64–240 像素，默认 112；
- **音效**：开关、音效组（`duck` / `fx1`）和音量。

设置保存在 `settings.json` 的 `agentPet` 字段。旧版「用量挂件」开着的用户升级后，桌宠会自动开启；
旧挂件的余额、价格、用量统计已经移除，相关设置在下次保存时清理。

## 图片

图片不随应用打包，运行时联网下载：

- 内置图库按固定提交从 jsDelivr 拉取，依次尝试 `cdn.jsdelivr.net`、`fastly.jsdelivr.net`、`gcore.jsdelivr.net`；
- 自定义图片只接受 `https://` 地址，不能带用户名密码，也不能指向本机或内网地址；
- 只接受 PNG / JPEG / WebP / GIF，会同时检查响应类型和文件头，单张不超过 4 MiB，跳转最多 3 次且必须仍是 https。

下载成功的图片缓存在 `<状态目录>/agent-pet-cache`，文件名是图片地址的 sha256 加扩展名。命中缓存时不再联网；
删掉这个目录会在下次显示时重新下载。管理器里的图库预览和 Codex 里的桌宠共用这份缓存。

### 图片来源与授权

内置图库来自微软 [Fluent Emoji](https://github.com/microsoft/fluentui-emoji) 的 3D 版本，MIT 许可，
固定提交 `1ffb34c752ecf5d402f04cfb4b392c77f57c54bc`。图片只在用户机器上运行时下载，本仓库不包含这些图片。

自定义图片由用户自己提供，请确认你有权使用。

## 音效

两组按压 / 松开音效随启动器内嵌，不需要联网。来源与授权见
[`assets/inject/agent-pet/sounds/README.md`](../assets/inject/agent-pet/sounds/README.md)。

## 实现位置

- 设置：`crates/codex-plus-core/src/settings.rs` 的 `AgentPetSettings`；
- 图库与下载：`crates/codex-plus-core/src/agent_pet.rs`；
- 渲染进程取图：桥接路由 `/agent-pet/image`（内置功能专用，不开放给第三方拓展）；
- 管理器命令：`agent_pet_catalog`、`agent_pet_image`；
- 渲染进程界面：`assets/inject/renderer-inject/97-agent-pet.js`。
