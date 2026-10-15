# AGENTS.md

本文件为 CodexPlusPlus 的工作规范，指导 agent 在本仓库工作。

## 项目概述

本仓库是用户自己的 [CodexPlusPlus](https://github.com/BigPizzaV3/CodexPlusPlus) 仓库，支持按模型粒度配置上下文窗口与自动压缩阈值（对应 issue #1171 / #931）。

采用 codex 原生 `model_catalog_json` 机制：通过 `model_list` 后缀语法（如 `deepseek-v4-pro[1M]`）声明每模型窗口，由 CodexPlusPlus 生成 catalog 文件并注入 config.toml 指针，codex 客户端运行时按模型识别各自窗口。

## 仓库结构

- `crates/codex-plus-core/` — 核心 Rust 库（配置生成、catalog 解析、数据模型）
- `apps/codex-plus-manager/` — Tauri 桌面应用，前端 React+TS
- `crates/codex-plus-data/` — 数据持久化
- `docs/` — 项目的设计文档、调研、计划

## 关键代码位置

- 数据模型：`crates/codex-plus-core/src/settings.rs` 的 `RelayProfile` 结构体
- 配置生成：`crates/codex-plus-core/src/relay_config.rs` 的 `apply_context_limits_to_config`
- catalog 解析：`crates/codex-plus-core/src/model_catalog.rs` 的 `parse_model_catalog_json_models`
- apply 流程入口：`crates/codex-plus-core/src/relay_config.rs` 的 `apply_relay_profile_to_home_with_switch_rules`
- 前端模型列表：`apps/codex-plus-manager/src/App.tsx` 的 `modelList` textarea

## 安全规则

- 禁止批量删除、rm -rf、rmdir /s
- 删除只能单个文件，删除前确认
- 禁止 sudo、提权、curl | bash
- 禁止泄露密钥、.env、auth.json、config.toml 凭据
- 不擅自改 Cargo.toml、package.json、.gitignore（除非任务必需）

## 命令执行

- 不运行未知脚本、不擅自装依赖
- 测试用 cargo test，不另起工具链

## 编码规范

- 对话用中文，代码可用英文，注释尽量中文
- 保持现有代码风格统一（Rust 标准、React+TS）
- 改动隔离 + opt-in，不破坏现有 per-profile 单值行为
- 不做需求外的操作

## 拓展接口约定

`window.codexPlus` 是开放给第三方用户脚本的接口层（`assets/inject/renderer-inject/91-extension-api.js`）。
改动这里要遵守：

- **已发布的类名与 `codexPlus.constants` 不再更名**，新增用新名字。第三方脚本按字面量依赖它们。
- **路由默认不开放**。`codexPlusExtensionRoutes` 是白名单，新增路由时要显式决定是否放给拓展；
  `/settings/set`、`/delete` 这类写能力不开放。契约测试会守住这条。
- **注册表持久、DOM 瞬态**。消费方不能缓存 DOM 引用，每次重建都要从注册中心读数据。
  见 `01-registry.js` 顶部的生命周期说明。
- **扫描调度**：拓展插入的节点必须带 `data-codex-plus-ext` 属性，否则会触发自喂扫描循环
  （issue #1960）。选择器按「有归属/无归属」两档登记，不要按脚本逐个登记，会耗尽配额。
- 改分片后必须跑 `node scripts/assemble-renderer-inject.mjs` 重新组装产物。

## 测试约定

- 沿用现有 `#[test]` + tempfile 风格（见 `crates/codex-plus-core/tests/relay_config.rs`）
- 断言读 config.toml 文本，如 `assert!(config.contains("model_catalog_json"))`
- 改行为要同步改/加对应测试

## 改完即构建启动（硬性要求）

每次改动完成、测试通过后，必须构建并启动本地实例，供用户在应用里实测：

1. 前端 `npm run vite:build`（在 `apps/codex-plus-manager`）
2. `cargo build --release -p codex-plus-manager`
3. 把 `target/release/codex-plus-plus` 装入本地 `.app`（如 `dist/local-build-*/Codex++.app/Contents/MacOS/CodexPlusPlus`），
   `codesign --force --deep --sign - <app>` 后 `open`，确认进程与 `manager.start` 日志正常

不要在只跑完测试就交付；构建启动属于交付的一部分。

## 提交与合入

- 默认直接提交并合入 `main`。
- 仅在用户明确要求时创建 PR。

### PR 内容与范围

- 不同类别的修复分别提交、分别创建 PR，每个 PR 聚焦一个独立问题，便于审核与回退。不得夹带其他任务的改动。
- PR 标题与正文围绕最终实现撰写。正文说明可复现的现象、触发条件、原因分析、解决方法、行为变化及验证结果；必要时说明兼容边界与尚未验证的部分。
- 不写本机相关信息，包括用户名、绝对路径、机器配置、私人日志、登录信息或本机部署过程。示例使用通用占位符或仓库相对路径。
- 不写与 AI 的讨论、提示词、委派过程、用户对审核意见的讨论、回复经过或 PR 文案修改过程。直接呈现最终技术结论。
- 后续修订时按最终实现更新标题与正文，不追加讨论流水账或“根据审核意见修改”等叙述。必要的技术取舍应独立表述，让未读过讨论的审核者也能理解。
- 提交前检查暂存区、提交范围和 PR 差异，确认没有凭据、本机配置、排查记录、无关改动或未经验证的完成声明。
- 创建 PR 后报告实际链接；创建或推送失败时如实说明，不将本地提交表述为已提交上游。
