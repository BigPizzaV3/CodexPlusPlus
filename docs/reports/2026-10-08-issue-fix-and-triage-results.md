# CodexPlusPlus 最新 100 个 issue 处置结果

日期：2026-10-08，北京时间。范围为本次审核的 #2257 至 #2414，共 100 条。

修复已直接合入并推送 main：[c07a337d](https://github.com/BigPizzaV3/CodexPlusPlus/commit/c07a337dcd1ab4424908b75844e290e5d1dc20ec)，从 1db62e92 起共 9 笔提交。先前 PR 随提交进入 main 被 GitHub 自动标记合并，聊天中的 PR 附件已移除。

## 处置统计

- 49 条针对性回复，覆盖 49 个不同 issue，0 失败。
- 关闭 7 条：#2410、#2403、#2399、#2394、#2391、#2297、#2259。
- 重开 3 条：#2408、#2385、#2339。
- 本批最终状态：OPEN 23、CLOSED 77。
- 新修复尚未发版，正式 v1.6.0 安装包仍不包含本次提交。

## 代码交付

| issue | 改动 | 提交 |
| --- | --- | --- |
| #2410、#2376 | 保留 macOS 原生签名包；Windows 首次安装与升级区分，尊重已删除图标 | ff517796 |
| #2391 | 白名单透传会话头，用户显式头优先，旧接口兼容 | d8334035、d5d09132 |
| #2399 | 抓取 Debugger.enable 局部超时、有限重试、过期代际退出 | f998e3c4 |
| #2259 | 管理器脚本整体开关，保留单项选择并重载 | 91e67eac |
| #2402 | 辅助读取 2 秒上限，迟到结果不应用，普通请求不等待目录 | edff1519 |
| #2339 | 主题包兼容级别与产品版本独立，准确提示不兼容 | a5abe072 |
| #2406 | 队列关闭时禁用 RPM 输入，说明完整响应串行语义 | d45ba20a |
| 语音测试 | 不锁死函数参数签名，完整语音脚本纳入 Cargo 回归 | c07a337d |

#2402 的原始续发故障、#2339 的 26.* 包契约和 #2406 的独立限速/并发需求继续开放。新布局、工作空间设置失败、沙盒初始化和 Gemini schema 等均已回复具体取证要求。

## 验证

最终 cargo test --offline --workspace：1787 passed、0 failed。使用 macOS 临时目录真实路径；默认 /var 别名下两条 Linked paths 失败已在原始基线复现，没有跳过或放宽安全检查。

TypeScript 检查、Vite 生产构建、renderer 分片组装一致性和 git diff --check 均通过。Windows COM/NSIS 实机安装与正式签名包实机操作仍有验证边界。[main 平台构建](https://github.com/BigPizzaV3/CodexPlusPlus/actions/runs/37723554515)在推送后自动运行。

## 逐条记录

| issue | 原审核分类 | 最终状态 | 本轮结果 | 沟通 |
| --- | --- | --- | --- | --- |
| [#2414](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2414) | 待定位 | OPEN | 已回复或更正，保留状态 | [回复](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2414#issuecomment-6051677370) |
| [#2413](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2413) | 待定位 | OPEN | 已回复或更正，保留状态 | [回复](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2413#issuecomment-6051530575) |
| [#2412](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2412) | 待定位 | OPEN | 已回复或更正，保留状态 | [回复](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2412#issuecomment-6051533336) |
| [#2410](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2410) | 待修 | CLOSED | 代码已修并推送 | [回复](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2410#issuecomment-6051663937) |
| [#2409](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2409) | 待定位 | OPEN | 已回复或更正，保留状态 | [回复](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2409#issuecomment-6051536868) |
| [#2408](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2408) | 待定位 | OPEN | 重新打开待定位 | [回复](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2408#issuecomment-6051588395) |
| [#2407](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2407) | 待定位 | OPEN | 已回复或更正，保留状态 | [回复](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2407#issuecomment-6051538739) |
| [#2406](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2406) | 部分处理 | OPEN | 部分修复，继续跟踪 | [回复](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2406#issuecomment-6051676399) |
| [#2405](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2405) | 咨询或需求 | CLOSED | 保留既有处置 | - |
| [#2404](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2404) | 待定位 | OPEN | 已回复或更正，保留状态 | [回复](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2404#issuecomment-6051540741) |
| [#2403](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2403) | 已修待验证 | CLOSED | 核对依据后关闭 | [回复](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2403#issuecomment-6051585549) |
| [#2402](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2402) | 待定位 | OPEN | 部分修复，继续跟踪 | [回复](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2402#issuecomment-6051674563) |
| [#2401](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2401) | 已修待验证 | CLOSED | 保留既有处置 | - |
| [#2400](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2400) | 待定位 | OPEN | 已回复或更正，保留状态 | [回复](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2400#issuecomment-6051542761) |
| [#2399](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2399) | 部分处理 | CLOSED | 代码已修并推送 | [回复](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2399#issuecomment-6051669475) |
| [#2398](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2398) | 咨询或需求 | OPEN | 已回复或更正，保留状态 | [回复](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2398#issuecomment-6051559409) |
| [#2397](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2397) | 已修待验证 | CLOSED | 保留既有处置 | - |
| [#2396](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2396) | 已修待验证 | CLOSED | 保留既有处置 | - |
| [#2395](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2395) | 咨询或需求 | CLOSED | 已回复或更正，保留状态 | [回复](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2395#issuecomment-6051577626) |
| [#2394](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2394) | 已修待验证 | CLOSED | 核对依据后关闭 | [回复](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2394#issuecomment-6051582902) |
| [#2393](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2393) | 待定位 | OPEN | 已回复或更正，保留状态 | [回复](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2393#issuecomment-6051544691) |
| [#2392](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2392) | 咨询或需求 | OPEN | 已回复或更正，保留状态 | [回复](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2392#issuecomment-6051555445) |
| [#2391](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2391) | 待修 | CLOSED | 代码已修并推送 | [回复](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2391#issuecomment-6051667599) |
| [#2390](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2390) | 待定位 | CLOSED | 已回复或更正，保留状态 | [回复](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2390#issuecomment-6051560621) |
| [#2385](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2385) | 待定位 | OPEN | 重新打开待定位 | [回复](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2385#issuecomment-6051591425) |
| [#2384](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2384) | 待定位 | OPEN | 已回复或更正，保留状态 | [回复](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2384#issuecomment-6051545769) |
| [#2380](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2380) | 待定位 | OPEN | 已回复或更正，保留状态 | [回复](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2380#issuecomment-6051547565) |
| [#2379](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2379) | 待定位 | CLOSED | 已回复或更正，保留状态 | [回复](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2379#issuecomment-6051562309) |
| [#2377](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2377) | 待定位 | CLOSED | 保留既有处置 | - |
| [#2376](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2376) | 待修 | CLOSED | 代码已修并推送 | [回复](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2376#issuecomment-6051665691) |
| [#2375](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2375) | 咨询或需求 | OPEN | 已回复或更正，保留状态 | [回复](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2375#issuecomment-6051556503) |
| [#2374](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2374) | 部分处理 | CLOSED | 已回复或更正，保留状态 | [回复](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2374#issuecomment-6051574059) |
| [#2373](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2373) | 部分处理 | CLOSED | 保留既有处置 | - |
| [#2372](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2372) | 部分处理 | CLOSED | 已回复或更正，保留状态 | [回复](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2372#issuecomment-6051572035) |
| [#2369](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2369) | 部分处理 | CLOSED | 保留既有处置 | - |
| [#2367](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2367) | 已修待验证 | CLOSED | 保留既有处置 | - |
| [#2364](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2364) | 已修待验证 | CLOSED | 保留既有处置 | - |
| [#2363](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2363) | 已修待验证 | CLOSED | 保留既有处置 | - |
| [#2362](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2362) | 已修待验证 | CLOSED | 已回复或更正，保留状态 | [回复](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2362#issuecomment-6051593280) |
| [#2360](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2360) | 已修待验证 | CLOSED | 保留既有处置 | - |
| [#2359](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2359) | 已修待验证 | CLOSED | 保留既有处置 | - |
| [#2358](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2358) | 已修待验证 | CLOSED | 保留既有处置 | - |
| [#2357](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2357) | 咨询或需求 | CLOSED | 保留既有处置 | - |
| [#2355](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2355) | 已修待验证 | CLOSED | 保留既有处置 | - |
| [#2354](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2354) | 已修待验证 | CLOSED | 已回复或更正，保留状态 | [回复](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2354#issuecomment-6051595167) |
| [#2351](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2351) | 已修待验证 | CLOSED | 保留既有处置 | - |
| [#2350](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2350) | 待定位 | OPEN | 已回复或更正，保留状态 | [回复](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2350#issuecomment-6051549487) |
| [#2349](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2349) | 待定位 | OPEN | 已回复或更正，保留状态 | [回复](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2349#issuecomment-6051551453) |
| [#2345](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2345) | 已修待验证 | CLOSED | 保留既有处置 | - |
| [#2344](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2344) | 咨询或需求 | CLOSED | 保留既有处置 | - |
| [#2343](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2343) | 已修待验证 | CLOSED | 保留既有处置 | - |
| [#2341](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2341) | 已修待验证 | CLOSED | 保留既有处置 | - |
| [#2340](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2340) | 部分处理 | OPEN | 已回复或更正，保留状态 | [回复](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2340#issuecomment-6051553379) |
| [#2339](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2339) | 待修 | OPEN | 部分修复，继续跟踪 | [回复](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2339#issuecomment-6051672656) |
| [#2338](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2338) | 已修待验证 | CLOSED | 保留既有处置 | - |
| [#2336](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2336) | 已修待验证 | CLOSED | 保留既有处置 | - |
| [#2332](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2332) | 已修待验证 | CLOSED | 保留既有处置 | - |
| [#2331](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2331) | 待定位 | CLOSED | 已回复或更正，保留状态 | [回复](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2331#issuecomment-6051565728) |
| [#2330](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2330) | 已修待验证 | CLOSED | 保留既有处置 | - |
| [#2329](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2329) | 部分处理 | CLOSED | 保留既有处置 | - |
| [#2328](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2328) | 咨询或需求 | CLOSED | 保留既有处置 | - |
| [#2327](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2327) | 已修待验证 | CLOSED | 保留既有处置 | - |
| [#2326](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2326) | 已修待验证 | CLOSED | 保留既有处置 | - |
| [#2325](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2325) | 待定位 | CLOSED | 已回复或更正，保留状态 | [回复](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2325#issuecomment-6051566906) |
| [#2324](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2324) | 已修待验证 | CLOSED | 保留既有处置 | - |
| [#2323](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2323) | 已修待验证 | CLOSED | 保留既有处置 | - |
| [#2322](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2322) | 待定位 | CLOSED | 已回复或更正，保留状态 | [回复](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2322#issuecomment-6051564033) |
| [#2321](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2321) | 咨询或需求 | CLOSED | 保留既有处置 | - |
| [#2315](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2315) | 待定位 | OPEN | 已回复或更正，保留状态 | [回复](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2315#issuecomment-6051554410) |
| [#2314](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2314) | 已修待验证 | CLOSED | 保留既有处置 | - |
| [#2312](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2312) | 咨询或需求 | OPEN | 已回复或更正，保留状态 | [回复](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2312#issuecomment-6051558306) |
| [#2310](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2310) | 已修待验证 | CLOSED | 保留既有处置 | - |
| [#2308](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2308) | 已修待验证 | CLOSED | 保留既有处置 | - |
| [#2306](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2306) | 已修待验证 | CLOSED | 保留既有处置 | - |
| [#2304](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2304) | 已修待验证 | CLOSED | 保留既有处置 | - |
| [#2302](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2302) | 已修待验证 | CLOSED | 保留既有处置 | - |
| [#2300](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2300) | 咨询或需求 | CLOSED | 保留既有处置 | - |
| [#2299](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2299) | 已修待验证 | CLOSED | 保留既有处置 | - |
| [#2298](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2298) | 咨询或需求 | CLOSED | 保留既有处置 | - |
| [#2297](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2297) | 待定位 | CLOSED | 核对依据后关闭 | [回复](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2297#issuecomment-6051580383) |
| [#2295](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2295) | 咨询或需求 | CLOSED | 保留既有处置 | - |
| [#2294](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2294) | 部分处理 | CLOSED | 已回复或更正，保留状态 | [回复](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2294#issuecomment-6051600431) |
| [#2289](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2289) | 咨询或需求 | CLOSED | 保留既有处置 | - |
| [#2287](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2287) | 部分处理 | CLOSED | 保留既有处置 | - |
| [#2284](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2284) | 已修待验证 | CLOSED | 保留既有处置 | - |
| [#2279](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2279) | 已修待验证 | CLOSED | 已回复或更正，保留状态 | [回复](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2279#issuecomment-6051596986) |
| [#2275](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2275) | 已修待验证 | CLOSED | 保留既有处置 | - |
| [#2273](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2273) | 待定位 | CLOSED | 已回复或更正，保留状态 | [回复](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2273#issuecomment-6051568135) |
| [#2270](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2270) | 待定位 | CLOSED | 已回复或更正，保留状态 | [回复](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2270#issuecomment-6051569013) |
| [#2269](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2269) | 咨询或需求 | CLOSED | 保留既有处置 | - |
| [#2268](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2268) | 咨询或需求 | CLOSED | 保留既有处置 | - |
| [#2267](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2267) | 已修待验证 | CLOSED | 保留既有处置 | - |
| [#2266](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2266) | 已修待验证 | CLOSED | 保留既有处置 | - |
| [#2264](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2264) | 已修待验证 | CLOSED | 已回复或更正，保留状态 | [回复](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2264#issuecomment-6051598306) |
| [#2263](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2263) | 已修待验证 | CLOSED | 已回复或更正，保留状态 | [回复](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2263#issuecomment-6051599981) |
| [#2261](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2261) | 已修待验证 | CLOSED | 已回复或更正，保留状态 | [回复](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2261#issuecomment-6051575879) |
| [#2260](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2260) | 待定位 | CLOSED | 已回复或更正，保留状态 | [回复](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2260#issuecomment-6051570953) |
| [#2259](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2259) | 部分处理 | CLOSED | 代码已修并推送 | [回复](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2259#issuecomment-6051670698) |
| [#2258](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2258) | 已修待验证 | CLOSED | 保留既有处置 | - |
| [#2257](https://github.com/BigPizzaV3/CodexPlusPlus/issues/2257) | 已修待验证 | CLOSED | 保留既有处置 | - |
