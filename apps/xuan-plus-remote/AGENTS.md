# 轩++远程端

本目录是独立源码副本，不是子模块；源码和运行不依赖原 WorkAgents 工作目录。
来源为 WorkAgents 提交 `27589048261db42d7aee44e361d60a1c3b7c8302`
下的 `harmony6-workagents-remote-app`。仅引入受版本控制的手机端、协议和云端源码，
未引入旧部署脚本、历史设计文档、运行数据、签名或本机配置。

## 目录与边界

- `app/`：HarmonyOS ArkTS/ArkUI 客户端。按用户明确授权，沿用 `com.dyys.workagents.remote.dev` 和原签名，覆盖手机上的原 WorkAgents；界面名称为轩++远程。
- `protocol/`：协议 1.5、夹具和参考模型。保留原协议签名域与 HTTP 字段，不能为了改名破坏协议。
- `environment/`：本副本的服务地址与一致性检查；按用户授权沿用现有 WorkAgents 云服务，不另建环境，不自动修改云端部署。
- `cloud-service/`：独立 Cargo 工作区，不参与轩++桌面默认构建；进程配置仅接受 `XUANPLUS_REMOTE_*`。
- 桌面接入位于 `crates/codex-plus-core/src/remote_mobile/` 和管理器“手机连接”页，使用本机确认绑定、Windows DPAPI 设备身份保护及按需选择的官方任务同步。已绑定手机仅可对已同步任务发送文本、新建同工作区任务和停止当前任务；其他远程执行仍拒绝。协议 1.5 的解绑通过每 30 秒复核绑定处理，不能把本地集成测试当作手机真机同步成功。

## 修改与验证

- 仅复用用户授权的应用签名和手机已有 HUKS 设备密钥别名、本机绑定选择；不得导出设备私钥、读取无关凭据和运行数据，不修改原项目及其部署环境。
- 两台开发机工具路径（2026-09-07 已验证）：
  - E 盘机器：`E:\Program Files\Huawei\DevEco Studio`
  - C 盘机器：`C:\Users\40448\DevEco Studio`
  - DevEco Studio、构建 SDK 与 HDC 默认自动检测：显式参数和环境变量优先；否则先检测 E 盘机器，再回退当前用户目录及 `C:\Program Files`。
  - 签名 HAP：`app/entry/build/default/outputs/default/entry-default-signed.hap`
- 构建：直接运行 `app/build-dev.ps1` 使用自动检测，也可显式传入 `-DevEcoRoot` / `-HarmonySdkRoot`；编译使用 DevEco 自带 SDK，最低兼容及目标版本保留 API 20；只用原装工具链，依赖缺失时不得擅自安装。
- 签名构建使用 `app/build-dev.ps1 -SigningConfigSource <原项目的 build-profile.json5>`；仅引用外部证书与密钥文件。构建时的本地忽略配置必须在退出时原样恢复，并由 `clear-signing-cache.ps1` 移除 Hvigor 生成的单个 `task-cache.json`；签名内容不得进入版本控制、命令行和日志。
- 开发 HAP 安装仅用 `app/install-dev.ps1`；只接受本目录的已签名 HAP，严格校验原开发 Bundle。仅通过 `install -r` 覆盖，不卸载、不清空手机数据。
- 签名缺失或不匹配时停止；不得改用 SDK 公共开发签名绕过校验。`sign-dev.ps1` 只委托上述原装签名构建入口。
- 独立性检查：`node verify-fork.mjs`；协议检查：`node protocol/verify-contract.mjs` 与 `node protocol/verify-reference-harness.mjs`。
- 服务配置检查：`node environment/verify-shared-service-config.mjs`。
- 云端测试：`cargo test --manifest-path cloud-service/Cargo.toml --offline --locked`。
- 测试使用内存或临时数据；运行态验证另行记录真实设备、连接和完整回复结果，所有诊断脱敏。
