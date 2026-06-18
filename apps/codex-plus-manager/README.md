# Codex++ Manager - 模型映射功能

## 概述

本目录包含 Codex++ Manager 的源代码和更新构建脚本。主要功能是在 Manager 的供应商配置 UI 中支持 **模型名称映射**，让 `spawn_agent` 请求中的固定模型名（如 `gpt-5.5`）被自动重写为上游供应商的实际模型名（如 `Lucis/mimo-v2.5-pro`）。

## 文件说明

| 文件 | 说明 |
|------|------|
| `update-and-build.bat` | 一键更新 + 编译脚本：git pull → 检查源码补丁 → 构建前端+后端 → 输出 exe |
| `patch-settings.ps1` | PowerShell 补丁脚本：向 `settings.rs` 添加 `model_mappings` / `model_mappings_enabled` 字段 |
| `patch-proxy.ps1` | PowerShell 补丁脚本：向 `protocol_proxy.rs` 添加模型名称重写逻辑 |
| `src/App.tsx` | Manager UI：供应商编辑界面中新增"是否映射原本模型"开关 + 下拉选择映射目标 |
| `src/styles.css` | 映射 UI 的样式 |

## 更新流程

双击 `update-and-build.bat`，自动执行：

1. **git pull** — 暂存本地修改 → 拉取远程更新 → 恢复本地修改
2. **检查 settings.rs** — 确认模型映射字段存在，缺失则自动补丁
3. **检查 protocol_proxy.rs** — 确认模型重写逻辑存在，缺失则自动补丁
4. **构建前端** — `npx vite build` 编译 Manager Web UI
5. **编译 Rust** — 按顺序编译 `codex-plus-core` → `codex-plus-launcher` → `codex-plus-manager`
6. **复制产物** — 将 `codex-plus-plus.exe` 和 `codex-plus-plus-manager.exe` 输出到本目录

## 使用方式

1. 打开 Manager，进入供应商配置
2. 选择或创建一个 API 供应商
3. 在底部找到 **模型映射** 区域：
   - 勾选"是否映射原本模型"启用映射
   - 点击"从上游获取可选模型"加载供应商支持的模型列表
   - 为每个 Codex 模型名选择对应的上游模型

## 架构原理

```
Codex CLI 请求 (model=gpt-5.5)
         │
         ▼
  Codex++ 协议代理 (127.0.0.1:57321)
         │
         ├─ upstream_request_parts()
         │   ├─ 检查 model_mappings_enabled
         │   ├─ 查找 model 字段是否在映射表中
         │   └─ 如果匹配则重写 model 值
         │
         ▼
  上游 API (localhost:3000)
         │
         ▼
  响应返回
```

## 修改的源码文件

| 文件 | 位置 | 修改内容 |
|------|------|----------|
| `settings.rs` | `crates/codex-plus-core/src/` | 添加 `model_mappings` (HashMap) 和 `model_mappings_enabled` (bool) 字段 |
| `protocol_proxy.rs` | `crates/codex-plus-core/src/` | 在 `upstream_request_parts` 中添加模型名称重写逻辑 |
| `App.tsx` | `apps/codex-plus-manager/src/` | 添加模型映射 UI：开关 + 下拉选择 |
| `styles.css` | `apps/codex-plus-manager/src/` | 映射 UI 样式 |

## 注意事项

- 如果 git pull 后 `git stash pop` 失败（代码冲突），需要手动解决冲突
- 冲突通常发生在 `settings.rs`、`protocol_proxy.rs`、`App.tsx` 这三个文件
- 修改后的 Manager 需要先停止旧进程，替换 exe 后再重新启动
- `.bat` 文件以 GBK 编码保存，不含中文，兼容中文 Windows cmd.exe