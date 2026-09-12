# 手机端签名构建与安装

最后验证：2026-09-11（Windows / PowerShell 7 / HarmonyOS 真机）。

本项目只接受开发 Bundle `com.dyys.workagents.remote.dev` 的已签名 HAP。
不要安装 `entry-default-unsigned.hap`，也不要卸载手机上的应用或清空其数据。

## 前置条件

- 手机已通过 HDC 连接，且开发者模式和调试授权已开启。
- 已安装 DevEco Studio 与 HarmonyOS SDK；脚本会自动发现本机工具链。
- 可访问原开发工程中与该 Bundle 对应的 `app/build-profile.json5`，以及其中引用的证书、profile 和 keystore 文件。
- 使用 PowerShell 7（`pwsh.exe`）。当前 `install-dev.ps1` 的 UTF-8 源码不应使用 Windows PowerShell 5.1 调用。

签名配置只作为临时引用传给构建脚本；请勿复制证书、密钥、密码或签名配置内容到本仓库、日志或命令行。

## 构建已签名 HAP

从仓库根目录运行。将 `<原开发工程>` 替换为保存原签名配置的工程根目录：

```powershell
pwsh.exe -NoProfile -File .\apps\xuan-plus-remote\app\build-dev.ps1 `
  -SigningConfigSource '<原开发工程>\app\build-profile.json5'
```

构建脚本会检查源工程与当前工程的 Bundle 一致性、验证签名材料存在，仅在本地临时写入签名引用，并在结束时恢复当前工程的 `build-profile.json5`。

成功产物固定为：

```text
apps\xuan-plus-remote\app\entry\build\default\outputs\default\entry-default-signed.hap
```

2026-09-11 已验证该流程生成：

```text
Bundle:      com.dyys.workagents.remote.dev
Version:     1.0.0
VersionCode: 1000010
```

## 覆盖安装并验证启动

构建成功后，仍从仓库根目录运行：

```powershell
$hap = '.\apps\xuan-plus-remote\app\entry\build\default\outputs\default\entry-default-signed.hap'
pwsh.exe -NoProfile -File .\apps\xuan-plus-remote\app\install-dev.ps1 -HapPath $hap
```

脚本会：

1. 拒绝非固定产物路径或 Bundle 不匹配的 HAP。
2. 要求唯一连接的 HDC 设备；多台设备时传入 `-TargetId <设备 ID>`。
3. 使用 `hdc install -r` 覆盖安装，不卸载、不清数据。
4. 启动 `EntryAbility`，并在 10 秒内确认应用进程或运行记录存在。

成功标志包含：

```text
installedBundle=com.dyys.workagents.remote.dev
installedTarget=redacted
processObserved=true
```

## 故障排查

只检查 HDC 工具路径：

```powershell
pwsh.exe -NoProfile -File .\apps\xuan-plus-remote\app\install-dev.ps1 -ResolveOnly
```

查看已连接设备：

```powershell
& '<HDC 路径>' list targets
```

若签名构建失败，核对原开发工程的 Bundle、默认签名配置及签名材料是否仍可访问。不要用公共开发签名、未签名 HAP 或不同 Bundle 的 HAP 绕过校验。
