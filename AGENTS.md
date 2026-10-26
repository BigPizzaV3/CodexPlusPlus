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
- 清理历史本地构建统一走 `node scripts/prune-local-builds.mjs --apply`（移入废纸篓，可恢复），不要自行 rm -rf
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

每次改动完成、测试通过后，必须构建并启动本地实例，供用户在应用里实测。

推荐一条命令：`node scripts/relaunch-local.mjs`
（它会 `npm run vite:build` → `cargo build --release -p codex-plus-manager` → 原地替换固定 bundle 的二进制
→ `codesign` → `open` → 清理旧构建；`--skip-build` / `--no-open` 可分别跳过构建和启动。）

固定 bundle 的规则：

- 固定使用 `dist/local-build/Codex++.app`，只替换 `Contents/MacOS/CodexPlusPlus`
- 保留 bundle 的 `Info.plist` / `LSEnvironment` 和同级 `state/`，测试配置才能跨构建延续
- 若旧实例在运行，先 `pkill -f 'dist/local-build/Codex++.app/Contents/MacOS'` 再替换
- 交付前清理历史构建：`node scripts/prune-local-builds.mjs --apply`（移入废纸篓，可恢复；relaunch 脚本会自动执行）

**不要再新建 `dist/local-build-<任务名>-<时间戳>` 目录，也不要按任务改名或改 bundle id。**
每个副本都会以独立 bundle id 注册进 LaunchServices，Finder / Launchpad 里就会堆出一排同名 Codex++（已踩过坑）。

`dist/local-build/Codex++.app` 丢失时，从 `/Applications/Codex++.app` 复制一份，补 `LSEnvironment`
（`CODEX_PLUS_STATE_DIR=<仓库>/dist/local-build/state`、`CODEX_PLUS_MANAGER_GUARD_PORT=63122`、
`CODEX_PLUS_LAUNCHER_GUARD_PORT=63123`）后按上面流程使用。

不要在只跑完测试就交付；构建启动属于交付的一部分。

## 提交与合入

- 默认直接提交并合入 `main`。
- 仅在用户明确要求时创建 PR。
- 提交信息、PR 描述里不加 `Co-Authored-By: Claude …` 等 AI 署名；本地 `.git/hooks/commit-msg` 也会自动删掉这一行。
