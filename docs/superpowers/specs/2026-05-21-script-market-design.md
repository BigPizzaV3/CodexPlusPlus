# 脚本市场设计

## 目标

把管理工具里的“用户脚本”页改造为“脚本市场”，基于 `https://github.com/BigPizzaV3/CodexPlusPlusScriptMarket` 提供的静态清单完成远程脚本浏览、安装、更新和本地状态展示。第一版不引入独立后端服务，市场仓库只需要托管 JSON 清单和脚本文件。

## 市场仓库格式

默认市场清单地址：

`https://raw.githubusercontent.com/BigPizzaV3/CodexPlusPlusScriptMarket/main/index.json`

清单版本为 `version: 1`，结构如下：

```json
{
  "version": 1,
  "updated_at": "2026-05-21T00:00:00Z",
  "scripts": [
    {
      "id": "demo-script",
      "name": "Demo Script",
      "description": "脚本用途说明",
      "version": "1.0.0",
      "author": "BigPizzaV3",
      "tags": ["ui", "productivity"],
      "homepage": "https://github.com/BigPizzaV3/CodexPlusPlusScriptMarket",
      "script_url": "https://raw.githubusercontent.com/BigPizzaV3/CodexPlusPlusScriptMarket/main/scripts/demo-script.js",
      "sha256": ""
    }
  ]
}
```

必填字段是 `id`、`name`、`version`、`script_url`。`description`、`author`、`tags`、`homepage`、`sha256` 可以为空或缺失。`sha256` 为空时跳过完整性校验；非空时必须匹配下载内容的 SHA-256 十六进制摘要。

## 后端设计

现有 `UserScriptManager` 继续负责本地脚本目录、启用状态和注入 bundle。新增脚本市场能力复用同一个用户脚本目录，不改变内置脚本和用户手动脚本的加载方式。

新增行为：

- 拉取远程市场清单并解析为稳定 JSON 结构。
- 过滤无效条目，保留有效条目和市场加载错误信息。
- 安装市场脚本时下载 `script_url` 指向的 JS 内容。
- 安装文件名固定为 `market-<id>.js`，避免覆盖用户手动放入的脚本。
- 安装成功后在 `user_scripts.json` 记录市场元数据，包括市场脚本 id、版本、来源 URL、homepage 和安装时间。
- 更新判断基于本地记录的版本和市场清单版本。版本字符串不同即显示“可更新”，不做语义化版本比较。
- 如果 `sha256` 校验失败，安装失败且不替换已有脚本。

`inventory()` 返回的本地脚本条目扩展字段：

- `market_id`
- `version`
- `installed`
- `source_url`
- `homepage`

原有字段 `key`、`name`、`source`、`enabled`、`status`、`error` 保持兼容。

## Tauri 命令设计

管理工具新增命令：

- `refresh_script_market`：拉取市场清单，返回市场条目、加载状态和当前本地 inventory。
- `install_market_script(id)`：根据市场清单中的 `id` 下载并安装脚本，返回更新后的市场状态和本地 inventory。

第一版不做后台缓存刷新。页面进入脚本市场时读取 settings 中已有的本地 inventory，再由用户点击刷新市场触发远程加载；如果网络失败，本地脚本列表仍可用。

## 前端设计

把导航和页面标题从“用户脚本”改为“脚本市场”。页面包含两个区域：

1. 市场概览和操作区：
   - 市场加载状态
   - 远程脚本数量
   - 已安装市场脚本数量
   - 用户脚本整体启用状态
   - 刷新市场按钮

2. 市场列表和本地列表：
   - 市场卡片展示名称、描述、版本、作者、标签和状态。
   - 状态包括“未安装”“已安装”“可更新”。
   - 操作包括安装、更新、打开主页。
   - 保留本地脚本列表，展示内置脚本、用户手动脚本和市场安装脚本的当前启用/关闭状态。

第一版不在管理工具里提供删除脚本或启停单个脚本，单脚本启停仍由注入菜单里的现有用户脚本控制保留。

## 错误处理

- 市场清单加载失败：前端展示错误消息，保留本地脚本列表。
- 清单 JSON 无效：命令返回失败状态和错误消息，不写入任何本地文件。
- 单个市场条目缺必填字段：过滤该条目，不影响其他条目。
- 下载失败：安装失败，不改变已有脚本。
- checksum 不匹配：安装失败，不替换已有脚本。
- 写入失败：返回失败状态，前端展示错误消息。

## 测试

Rust 测试覆盖：

- 市场清单解析和无效条目过滤。
- 安装市场脚本写入 `market-<id>.js`。
- 安装后 `user_scripts.json` 记录市场元数据。
- checksum 失败不替换已有脚本。
- inventory 能合并市场安装元数据。

前端验证：

- TypeScript check 通过。
- 构建通过或至少管理工具前端 check 通过。

## 非目标

- 不做脚本评分、评论、收藏。
- 不接 GitHub API。
- 不实现仓库端自动生成清单。
- 不提供脚本删除功能。
- 不修改 Codex App 原始文件。
