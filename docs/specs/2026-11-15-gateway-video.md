# 网关视频生成

## 目标

Codex++ 网关把视频模型作为独立的 Agent 能力暴露，不把视频模型混进聊天模型列表。任意接入网关的 Agent 都可以使用同一套 OpenAI 风格异步 Videos 接口，不需要绑定某个供应商或某个用户的订阅。

## 对外接口

所有请求使用该 Agent 已签发的网关 token：

| 方法 | 路径 | 说明 |
| --- | --- | --- |
| `POST` | `/gw/<agent>/v1/videos` | 创建视频任务 |
| `GET` | `/gw/<agent>/v1/videos/{id}` | 查询任务状态 |
| `GET` | `/gw/<agent>/v1/videos/{id}/content` | 下载视频文件 |

创建请求保持 OpenAI 风格 JSON，例如：

```json
{
  "model": "wan2.7-t2v",
  "prompt": "A paper boat drifting down a rain-soaked street",
  "duration": 5,
  "aspect_ratio": "16:9",
  "resolution": "1080p"
}
```

网关返回统一状态：

```json
{
  "id": "video_...",
  "object": "video",
  "model": "wan2.7-t2v",
  "status": "queued",
  "progress": 0,
  "created_at": 1763200000,
  "upstreamAdapter": "dashscope"
}
```

`status` 统一为 `queued`、`in_progress`、`completed` 或 `failed`。完成时可能同时带 `url`。

## 选择与路由

- 视频模型从供应商模型列表里按名称识别，例如 `video`、`sora`、`veo`、`kling`、`wan*-t2v`、`wan*-i2v`。
- 生图和视频都命中的名称按视频处理，避免 `wanx-video` 同时出现在两个列表。
- 在「Agent 接入」里，每个 Agent 可单独选择自动、指定 `<供应商 id>/<模型>` 或关闭。
- 自动模式按供应商池顺序每家尝试一次；创建成功后，本地任务 ID 固定到当时的供应商、Key 池和适配器，后续轮询不换家。
- 需要 Base URL 和 Key 的 API 供应商可直接使用；订阅登录账号暂不支持视频。

## 供应商适配器

| 适配器 | 创建 | 查询 | 说明 |
| --- | --- | --- | --- |
| OpenAI | `POST /videos` | `GET /videos/{id}` | 请求体基本原样透传，文件走 `/videos/{id}/content` |
| xAI | `POST /videos/generations` | `GET /videos/{id}` | 兼容 `request_id`、`pending/done/failed` 和 `video.url` |
| DashScope | `POST /api/v1/services/aigc/video-generation/video-synthesis` | `GET /api/v1/tasks/{id}` | 转换 input / parameters；Wan 2.7 使用 `resolution` + `ratio` |

适配器只根据 Base URL 和模型名选择，不读取或修改任何 Agent 的私有数据。

## 状态与限制

- 任务映射保存在 Codex++ 网关进程内；重启 Codex++ 后，旧任务 ID 不能继续轮询。
- 视频文件上限为 512 MiB，下载超时为 5 分钟。
- 仅代理公网 HTTP(S) 文件链接，拒绝 localhost 和字面量私网 IP。
- 这是一个通用网关能力，不包含 Accio、Qwen Token Plan 或其他客户端的专用逻辑。

## 回归范围

- 视频名称识别不会吞掉聊天或图片模型。
- 自动模式按供应商池顺序选择，显式选择和关闭都有明确错误。
- OpenAI、xAI、DashScope 的创建、查询和状态归一化路径有单元测试。
- `videoModels` 设置可持久化，并兼容错误写成数组的旧配置文件。
- Agent 接入、供应商详情、新手引导和模型测试都会排除视频模型。
