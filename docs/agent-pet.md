# Agent 桌宠

在 Codex 里放一只可拖动的桌宠。点击时会压扁再弹回，并播放按压 / 松开音效。

## 开启

打开管理器「Agent 桌宠」页，打开开关即可，默认关闭。目前只支持 Codex，其他 Agent 之后再补。

页面里可以设置：

- **图片**：从内置图库（含二次元形象）挑一张，或选「自定义」并填一个 `https://` 图片地址；
- **大小**：64–240 像素，默认 112；
- **音效**：开关、音效组和音量。音效组可选内嵌的两组、可下载的二次元音效，或导入本地音频作为自定义按压 / 松开音。

设置保存在 `settings.json` 的 `agentPet` 字段。旧版「用量挂件」开着的用户升级后，桌宠会自动开启；
旧挂件的余额、价格、用量统计已经移除，相关设置在下次保存时清理。

## 图片

图片不随应用打包，运行时联网下载：

- 内置图库按固定提交从 jsDelivr 拉取，依次尝试 `cdn.jsdelivr.net`、`fastly.jsdelivr.net`、`gcore.jsdelivr.net`；
- 二次元形象直接从 Wikimedia Commons 拉取，下载后核对登记的 sha256，上游文件被替换时拒绝使用；
- 自定义图片只接受 `https://` 地址，不能带用户名密码，也不能指向本机或内网地址；
- 只接受 PNG / JPEG / WebP / GIF，会同时检查响应类型和文件头，单张不超过 4 MiB，跳转最多 3 次且必须仍是 https。

下载成功的图片缓存在 `<状态目录>/agent-pet-cache`，文件名是图片地址的 sha256 加扩展名。命中缓存时不再联网；
删掉这个目录会在下次显示时重新下载。管理器里的图库预览和 Codex 里的桌宠共用这份缓存。

### 图片来源与授权

内置图库来自微软 [Fluent Emoji](https://github.com/microsoft/fluentui-emoji) 的 3D 版本，MIT 许可，
固定提交 `1ffb34c752ecf5d402f04cfb4b392c77f57c54bc`。图片只在用户机器上运行时下载，本仓库不包含这些图片。

二次元形象来自 Wikimedia Commons，均为 CC0（公有领域贡献）：

| id | 名称 | 作者 | 原始页面 |
|---|---|---|---|
| `chibi-bow` | 蝴蝶结 | asrafil | <https://commons.wikimedia.org/wiki/File:Chibiw.png> |
| `chibi-girl` | Q版少女 | dcatcherex | <https://commons.wikimedia.org/wiki/File:Cute_girl.png> |
| `anime-girl` | 马尾少女 | oksmith | <https://commons.wikimedia.org/wiki/File:Anime_girl_publicdomainq.png> |
| `anime-angry` | 炸毛少女 | snoopingasusual | <https://commons.wikimedia.org/wiki/File:Angry_anime_girl.png> |

自定义图片由用户自己提供，请确认你有权使用。

## 音效

音效有三种来源，设置里的 `soundSet` 决定用哪一种：

- **内嵌**（`duck` / `fx1`）：随启动器内嵌，不需要联网。来源与授权见
  [`assets/inject/agent-pet/sounds/README.md`](../assets/inject/agent-pet/sounds/README.md)。
- **可下载**：二次元风格的短音效，第一次选中时从 Freesound 的公开预览地址下载（MP3，单个不超过 2 MiB），
  和图片共用 `agent-pet-cache` 缓存。全部为 CC0：

  | id | 名称 | 按压 / 松开 | 作者 |
  |---|---|---|---|
  | `nya` | 猫娘「にゃ」 | [336012](https://freesound.org/people/Mike_bes/sounds/336012/) / 无 | Mike_bes |
  | `hai` | 元气「はい」 | [323194](https://freesound.org/people/Alivvie/sounds/323194/) / [323196](https://freesound.org/people/Alivvie/sounds/323196/) | Alivvie |
  | `yatta` | 欢呼「やった」 | [235085](https://freesound.org/people/Reitanna/sounds/235085/) / [235093](https://freesound.org/people/Reitanna/sounds/235093/) | Reitanna |
  | `ehehe` | 轻笑「えへへ」 | [343992](https://freesound.org/people/Reitanna/sounds/343992/) / [343984](https://freesound.org/people/Reitanna/sounds/343984/) | Reitanna |
  | `pop` | 啵唧 | [789793](https://freesound.org/people/quatricise/sounds/789793/) / [857748](https://freesound.org/people/qubodup/sounds/857748/) | quatricise / qubodup |
  | `sparkle` | 魔法闪光 | [618961](https://freesound.org/people/Hemplock/sounds/618961/) / [511485](https://freesound.org/people/MLaudio/sounds/511485/) | Hemplock / MLaudio |

- **自定义**（`custom`）：在管理器里导入本地 MP3 / OGG / WAV / M4A（不超过 2 MiB），文件按内容哈希命名后复制到
  `<状态目录>/agent-pet-custom`，文件名记在 `customSoundPress` / `customSoundRelease`。按压音和松开音可以只设一个；
  同一槽位重新导入会替换旧文件。设置里只接受 `press-<16 位十六进制>.<扩展名>` 这种形状的文件名。

## 实现位置

- 设置：`crates/codex-plus-core/src/settings.rs` 的 `AgentPetSettings`；
- 图库与下载：`crates/codex-plus-core/src/agent_pet.rs`；
- 渲染进程取图 / 取音效：桥接路由 `/agent-pet/image`、`/agent-pet/sound`（内置功能专用，不开放给第三方拓展）；
- 管理器命令：`agent_pet_catalog`、`agent_pet_image`、`agent_pet_sounds`、`agent_pet_sound_catalog`、
  `agent_pet_sound`、`agent_pet_import_sound`；
- 渲染进程界面：`assets/inject/renderer-inject/97-agent-pet.js`。
