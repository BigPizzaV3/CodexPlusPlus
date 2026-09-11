import assert from "node:assert/strict";
import { readFile, readdir, lstat } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.dirname(fileURLToPath(import.meta.url));
const read = (file) => readFile(path.join(root, file), "utf8");
const bundle = "com.dyys.workagents.remote.dev";
const app = JSON.parse(await read("app/AppScope/app.json5"));
assert.equal(app.app.bundleName, bundle, "覆盖安装必须沿用用户授权的原应用身份");
const product = JSON.parse(await read("app/build-profile.example.json5")).app.products[0];
assert.equal(product.compatibleSdkVersion, "6.0.0(20)", "最低兼容版本不能被迁移抬高");
assert.equal(product.targetSdkVersion, "6.0.0(20)", "目标版本应保持不变");
assert.equal(product.compileSdkVersion, undefined, "编译版本应由原装 DevEco SDK 决定");
const signing = await read("app/sign-dev.ps1");
const installing = await read("app/install-dev.ps1");
const push = await read("app/entry/src/main/ets/remote/PushNotificationCoordinator.ets");
for (const source of [installing, push]) {
  assert.ok(source.includes(`'${bundle}'`), "签名、安装和通知身份必须与应用一致");
  assert.ok(!source.includes("com.dyys.xuanplusplus.remote.dev"), "不得要求用户新建另一应用");
}
assert.match(signing, /SigningConfigSource/, "签名必须显式选择授权来源");
assert.ok(!signing.includes("OpenHarmony.p12"), "不能使用 SDK 公共密钥冒充原签名");
const build = await read("app/build-dev.ps1");
assert.match(build, /WriteAllBytes\(\$buildProfile, \$originalBuildProfile\)/, "签名后必须恢复本地配置");
assert.match(build, /finally/, "失败出口也必须恢复配置");
assert.match(build, /signingRedactions/, "签名日志必须脱敏");
assert.ok(build.includes("'clear-signing-cache.ps1'"), "构建退出时必须清理含签名配置的任务缓存");
assert.ok(build.includes("'E:\\Program Files\\Huawei\\DevEco Studio'"), "构建必须检测 E 盘工具链");
assert.match(build, /Join-Path \$userProfileDirectory 'DevEco Studio'/, "构建必须回退当前用户目录工具链");
assert.match(build, /\[IO\.Directory\]::Exists\(\$Candidate\)/, "不存在的候选盘符不能中断工具链检测");
assert.match(build, /if \(\$env:CODEX_CI -eq '1'\)/, "受限构建环境必须使用无目录联接入口");
const moduleAlias = await read("app/hvigor/deveco-module-alias.cjs");
assert.match(moduleAlias, /@ohos\/hvigor-ohos-plugin/, "无目录联接入口必须加载 DevEco 内置插件");
const cacheCleanup = await read("app/clear-signing-cache.ps1");
assert.ok(cacheCleanup.includes("'.hvigor/cache/task-cache.json'"), "只能清理明确的签名缓存文件");
assert.ok(!cacheCleanup.includes("-Recurse"), "禁止递归删除签名缓存目录");
assert.match(installing, /moduleProfile\.app\.bundleName/, "安装前必须核对 HAP 内嵌身份");
assert.match(installing, /processObserved=true/, "真机验证必须观察到应用进程");
assert.ok(installing.includes("'E:\\Program Files\\Huawei\\DevEco Studio\\sdk'"),
  "安装脚本必须检测 E 盘 HDC");
assert.match(installing, /Join-Path \$userProfileDirectory 'DevEco Studio\\sdk'/,
  "安装脚本必须回退当前用户目录 HDC");
const opening = await read("app/open-in-deveco.ps1");
assert.ok(opening.includes("'E:\\Program Files\\Huawei\\DevEco Studio\\bin\\devecostudio64.exe'"),
  "打开工程脚本必须检测 E 盘 DevEco Studio");
assert.match(opening, /Join-Path \$userProfileDirectory 'DevEco Studio\\bin\\devecostudio64\.exe'/,
  "打开工程脚本必须回退当前用户目录 DevEco Studio");
const identity = await read("app/entry/src/main/ets/remote/DeviceIdentityCoordinator.ets");
assert.ok(identity.includes("'workagents_remote_dev_ed25519_v3'"), "覆盖时必须保留手机已有设备身份");
const index = await read("app/entry/src/main/ets/pages/Index.ets");
assert.ok(index.includes("'workagents.remote.activePcDeviceKey'"), "覆盖时必须保留原绑定选择");
assert.match(index, /@State private expandedTaskWorkspaces: string\[\] = \[\]/,
  "任务项目必须默认折叠");
assert.match(index, /return runningGroups\.concat\(idleGroups\)/,
  "包含运行中任务的项目必须排在前面");
assert.match(index, /if \(workspaceGroup\.runningTaskCount > 0\)/,
  "项目名称后必须显示运行中任务数量角标");
assert.match(index, /if \(this\.taskWorkspaceIsExpanded\(workspaceGroup\.name\)\)/,
  "任务卡片必须仅在项目展开后显示");
const manifest = await read("cloud-service/Cargo.toml");
assert.match(manifest, /^\[workspace\]/m, "云端应使用本目录内的独立 Cargo 工作区");
assert.match(manifest, /name = "xuan-plus-remote-cloud"/);
const cloud = await read("cloud-service/src/main.rs");
assert.ok(!cloud.includes("WORKAGENTS_REMOTE_"), "云端不能读取原服务的进程配置");
assert.ok(cloud.includes("XUANPLUS_REMOTE_DATABASE"));
const config = JSON.parse(await read("environment/shared-remote-service.json"));
if (!config.enabled) {
  const mobile = await read("app/entry/src/main/ets/remote/RemoteEnvironmentConfig.ets");
  assert.match(mobile, /if \(!RemoteEnvironmentConfig.enabled\)/, "未配置服务时必须阻止网络请求");
  const ability = await read("app/entry/src/main/ets/entryability/EntryAbility.ets");
  assert.match(ability, /if \(RemoteEnvironmentConfig.enabled\)/, "未配置服务时不得启动推送接入");
}
// 仅检查源码目录，避免扫描依赖、签名和构建输出。
const sourceDirectories = ["app/AppScope", "app/entry/src", "protocol", "environment", "cloud-service/src"];
let count = 0;
async function verifyDirectory(directory) {
  for (const item of await readdir(directory, { withFileTypes: true })) {
    const file = path.join(directory, item.name);
    assert.equal((await lstat(file)).isSymbolicLink(), false, "独立副本不能通过链接依赖原仓库");
    if (item.isDirectory()) {
      await verifyDirectory(file);
    } else {
      const bytes = await readFile(file);
      const text = new TextDecoder("utf-8", { fatal: true }).decode(bytes);
      assert.ok(!text.includes(String.fromCharCode(0xfffd)), "源码包含替换字符");
      assert.ok(!text.includes("E:\\GIT_DYYS\\work-agents"), "不得依赖原项目绝对路径");
      count++;
    }
  }
}
for (const directory of sourceDirectories) await verifyDirectory(path.join(root, directory));
console.log(`独立源码、授权签名复用、安装边界与 ${count} 个源码文件检查通过`);
