# 管理器顶部品牌栏移除验证

final result: passed

按用户附件 `codex-clipboard-da4d79f0-be0f-4579-9953-3f37112108de.png` 完整移除管理器内容区顶部的 C++ / Codex++ 品牌条。删除其 JSX、专用样式、更新圆点与未使用的图标导入；外壳改为单行网格，移除40px预留高度，并同步内容区高度公式。

- 隔离预览证据：`/Users/mac/.codex/visualizations/2026/10/08/01a11b0a-e04d-7f31-bee4-672ad6ef5da1/manager-no-brand-header/preview.jpg`。
- 页面中品牌栏节点数量为0；单一网格行占满视口，图标栏、导航栏、工作区的top均为0。
- 普通页面与通用设置页均验证无顶部残留占位，导航功能正常；字体、现有图标和主题色沿用既有样式。
- 品牌栏未设置Tauri拖动区域，移除不涉及系统窗口标题栏。预览使用模拟数据，不操作真实配置。
- `npm run check`、前端生产构建和`git diff --check`通过；控制台没有error/warn。构建输出到临时目录，未清空已有构建目录。

无剩余P0/P1/P2问题；正式安装版需重新打包管理器后生效。

---

## 既有验证记录

# 自定义布局侧边栏整体移动验证

final result: passed

本轮修复用户截图中的背景残留与图标栏/会话栏脱离问题。原实现分别移动内层导航内容，未接管绘制背景、边框并占据原位置的外壳；现在使用一个 sidebar 布局单位接管完整原生外壳。无共同外壳时，两个原生外壳通过同一组几何一起移动；不重建或搬移 React 子树。

- 原始问题证据：用户附件 `codex-clipboard-3e215368-b7a9-419a-a5af-fa23245b5ec8.png`、`codex-clipboard-5ff58252-6c6f-4891-b773-c919c3342b59.png`。
- 浏览器复现使用实际96-layout-physics.js与96-custom-layout.js，以及共同外壳/独立外壳两种真实flex布局；会话数据是虚构数据，不连接实际Codex或修改本机配置。
- 截图：`/Users/mac/.codex/visualizations/2026/10/08/01a11b0a-e04d-7f31-bee4-672ad6ef5da1/custom-layout-sidebar/before.jpg`、`after.jpg`，882 × 720 CSS px。
- 拖动后主区左沿从314px变为0、宽度变为882px；侧边栏整体位于560–874px，rail宽54px，会话区相邻间隙为0，旧位置命中主区而非残留侧栏。
- 字体与图像沿用宿主及既有控件；修复只调整布局目标和几何，没有复制或伪造背景。背景、边框、滚动分工和实际DOM父子关系保留。
- 缩放共同外壳时同步内部两个子壳宽度，保持图标栏宽度，剩余宽度用于会话栏；已接管壳不会因为瞬时几何变化重新拆组。没有通过overflow裁剪掩盖溢出。
- 旧v1位置记录向grouped sidebar兼容迁移，聊天栏记录优先，rail-only也可恢复；保留原storage键，仅沿用既有交互保存点写回。
- 浏览器已验证共同/独立外壳拖动、连续缩放、节点重建、重置和关闭；关闭后恢复原生314px占位，相关inline样式完整释放。控制台未发现error/warn。
- `cargo test -p codex-plus-core --test custom_layout_runtime`的两个包装测试均通过，覆盖布局运行时、物理求解、设置与增强集成；新增回归包含背景外壳、连续缩放、子壳替换、旧记录迁移、取消和恢复。组装产物与分片一致，`git diff --check`通过。

没有剩余P0/P1/P2发现。验证为隔离复现，未操作正在运行的Codex；应用内生效需重新构建启动器。

---

## 既有页面和图标检查记录

# ChatGPT 与 Grok 切换图标核对

final result: passed

本轮仅调整管理器 Agent 切换栏中的品牌图标。参考为用户附件 `codex-clipboard-01e4f3f8-598d-4a24-99cb-1196b2505530.png`：箭头指向原 Codex 彩色方底图标。最终替换为 ChatGPT 使用的 OpenAI 标志，与既有 Grok 使用同源单色 SVG。来源及 MIT 许可已写入 `apps/codex-plus-manager/src/assets/agents/LICENSE.txt`。

- 实现证据：`/Users/mac/.codex/visualizations/2026/10/08/01a11b0a-e04d-7f31-bee4-672ad6ef5da1/manager-brand-icons/dark.jpg`、`light.jpg`；图标区域组合为 `icons-preview.jpg`。
- 字体与布局：工具名称、导航及文字样式保持既有行为；两枚图标均为28 × 28px，44px按钮与原有间距保留。
- 颜色与资产：统一使用原始 SVG 蒙版和 currentColor，无白色图标底、彩色渐变或单独反色；深色/浅色、选中、悬停及键盘焦点共享同一套样式。保持品牌原始路径与比例。
- 内容与交互：Codex/Grok的工具身份、按钮标签与切换功能保留。隔离预览已验证双向切换及主题，未操作真实配置。
- 验证：`npm run check`通过；生产前端构建通过，输出到临时目录并禁止清空目录；`git diff --check`通过。未新增低影响图标变更的自动化测试，未重新打包原生安装程序。

本轮未发现P0/P1/P2问题。原图用于确定替换位置，目标资产为已保存的ChatGPT/OpenAI SVG；检查重点为该图标与Grok在相同控件尺寸、主题与状态下的视觉一致性。

---

## 既有页面检查记录

# Codex++ 拓展与推荐页原生样式核对

final result: passed

## 范围与视觉证据

本轮对照用户提供的 Codex 原生插件页面，调整拓展与推荐内容页的布局和视觉风格。当前代码直接在隔离预览中执行；安装、启停、卸载测试只修改内存夹具，不操作真实脚本或本机配置。已安装启动器尚未更新。

- 视觉参考：`/var/folders/0s/y8fsqjhd20d_twbcc2j7mj040000gn/T/codex-clipboard-4e01003c-8d57-48c9-b3d5-ef2e26577871.png`，3440 × 1968 px，按密度 2 归一，再裁出 1663 × 936 的应用内容区。
- 原拓展与推荐页：用户附件 `codex-clipboard-334422ac-181e-4b75-9ede-13ddc70d5b0d.png`、`codex-clipboard-0f2f0bc7-6e5e-45e5-b27b-383466a77817.png`。
- 证据目录：`/Users/mac/.codex/visualizations/2026/10/08/01a11b0a-e04d-7f31-bee4-672ad6ef5da1/codex-plus-discovery/`。
- 最终拓展截图：`extensions-hires.png` / `extensions-dark.jpg`；推荐页：`sponsor-hires.png` / `sponsor-dark.jpg`。CSS 视口均为 1663 × 936，原截图 3326 × 1872，密度 2；JPEG 按密度归一到 CSS 尺寸。
- 明亮主题：`extensions-light-hires.png`、`sponsor-light-hires.png`。窄窗口：`extensions-narrow-light.jpg`、`sponsor-narrow-light.jpg`，360 × 800。
- 整体比较：`extensions-comparison.jpg`；文字、图标与列表局部：`extensions-detail-comparison.jpg`、`sponsor-comparison.jpg`。
- 原生页面的插件、技能、热门等文案属于原产品的数据与功能。本轮保留自己的市场、已安装、详情和合作伙伴内容，未添加无数据来源的分类或无功能的按钮。

## 五项视觉核对

- 字体与文字：继承宿主字体；页面标题、分组标题、名称和简介分别保持明确层级。拓展名称和单行简介截断后，详情仍提供完整内容。
- 布局与间距：312px 紧凑侧栏，主内容 900px 居中；两列列表，每列 434px；拓展条目 56px 高、行距 12px，36px 图标。搜索框与低调的刷新、安装按钮对齐。
- 色彩：主区深色回落值与参考同为 #181818，侧栏 #1e1e1e，选中背景 #2f2f2f；浅色主题与键盘焦点清晰可读。优惠文字取消大块橙色底。
- 图像：拓展继续使用市场图标或既有默认扩展字形；推荐页五个真实 Logo 均加载成功。图标、箭头与品牌图片保持比例，未创造或替换品牌标识。
- 文案与数据：保留真实市场清单、原始外链、简介、优惠文案、使用要求与限制；安装目录、脚本 key、启停数据属性与用户/内置卸载权限保持原契约。

## 比较与修正记录

1. 原页将市场条目放在大块侧栏，主区默认空白。改为简洁已安装侧栏与主区浏览列表，详情可返回列表。
2. 初版两行简介与较高行距使密度偏松，见 `extensions-initial.jpg`。最终改为单行简介、56px 条目与12px行距，按相同视口和像素密度重新核对整体与局部。
3. 搜索更新保留输入框 DOM；异步数据更新保留已聚焦的启停开关与筛选按钮。失效选择先归一化再同步侧栏和结果，安装后详情转为已安装状态。
4. 窄窗口改为单列列表与56px图标侧栏；两页未发生横向溢出。1.2 倍缩放下列表与两列布局正常。

## 交互与回归

预览已核对搜索与空结果、详情、返回、市场/已安装筛选、刷新、模拟安装/卸载、键盘启停和筛选焦点；控制台未发现 error / warn。广告原始 URL 与图片保留，五个 Logo 加载成功。

`cargo test -p codex-plus-core --test custom_layout_runtime custom_layout_renderer_integration_preserves_existing_enhancements` 通过，包含真实渲染函数的数据筛选、转义、独立按钮、搜索 DOM 和广告字段回归。`--test ads` 的7项测试与 `--test cdp_bridge injection_script_menu_` 的2项测试通过。分片组装与产物一致，`git diff --check` 通过。

没有剩余 P0 / P1 / P2 发现。完整应用内的效果需在重新构建启动器后核对；本轮未更新正在运行的应用。

---

## 既有设置页及导航检查记录

# Codex++ 设置页样式核对

final result: passed

## 范围与证据

本次对齐 Codex 原生设置页的视觉样式，保留 Codex++ 自身的设置内容与数据契约。
实现预览直接执行仓库中的页面模板、样式、开关刷新和下拉控制函数；后端使用固定数据，不写实际配置。
当前运行中的 Codex 原生窗口未做自动核对，也未更新已安装的启动器。

- 页面参考：`/var/folders/0s/y8fsqjhd20d_twbcc2j7mj040000gn/T/codex-clipboard-bc36411c-4466-4ab4-be61-63f5b26ea3e3.png`。
- 下拉参考：`/var/folders/0s/y8fsqjhd20d_twbcc2j7mj040000gn/T/codex-clipboard-05baab45-883e-4c00-b7fc-4c40035abaa6.png`。
- 证据目录：`/Users/mac/.codex/visualizations/2026/10/08/01a11b0a-e04d-7f31-bee4-672ad6ef5da1/codex-plus-settings/`。
- 页面实现：`dark-desktop.jpg`，1344 × 936 CSS px，截图密度 1。
- 整体与局部比较：`comparison.jpg`、`comparison-detail.jpg`。参考 3440 × 1968 px 按密度 2 归一到 1720 × 984，再裁出 1344 × 936 的内容区；不比较外层窗口与原生设置导航。
- 下拉实现：`dropdown-dark.jpg`，1280 × 720 CSS px；局部 `dropdown-detail.jpg`，354 × 180 px。
- 下拉比较：`dropdown-comparison.jpg`。参考 708 × 342 px 按密度 2 归一到 354 × 171，与同宽的触发器及展开菜单比较。
- 补充状态：`dark-narrow.jpg`、`dark-narrow-expanded.jpg`、`light-narrow.jpg`、`dropdown-light-narrow.jpg`、`dropdown-zoom.jpg`。

## 已解决的问题

1. 全宽设置列表改为 740px 居中栏、五个分组卡片和共同左边界的页面标题。
2. 初版单行高度约 69px，收紧后为 60–61px；卡片底色与原生参考的深色表面接近，标题与描述保留层级。
3. 窄窗口下服务模式控制区曾只有约 141px，修正选择器优先级后可使用完整 294px 行宽；360px 窗口无横向溢出。
4. 系统 select 弹出菜单改为主题浮层，使用圆角、选中勾和原生选项行节奏；初版菜单高 158px，收紧后为 126px。原 select 保留为隐藏数据载体。

## 五项视觉核对

- 字体：继承宿主系统字体；页面标题 28px，设置标题 14px，描述 13px，下拉选项 14px。窄窗口文字正常换行。
- 布局：页面顶部留白、48px 分组间距、16px 卡片圆角、细分隔线、32 × 20px 开关；下拉触发器 28px 高，菜单 220px 宽。1.2 倍缩放时菜单右边缘与按钮的差值为 0。
- 颜色：使用宿主语义令牌和按明暗主题区分的回落值；设置表面、控件、选中状态和焦点有清晰层级。
- 资产：保留既有状态点；新增箭头及勾选采用仓库已安装的 Lucide ChevronDown / Check 图标路径，没有新增栅格资产。
- 文案：保留原有 Codex++ 设置、四个打字特效选项、说明、链接和 Windows 条件项。原生文件打开应用图标与文案属于另一项设置，不复制到打字特效菜单。

## 交互与回归

- 开关状态、无障碍状态、后端等待与禁用状态同步。
- 服务模式行在关闭时隐藏，窄窗口展开时不溢出。
- 下拉点击选择仍经原 select 的冒泡 change 保存；重复选择不额外保存，禁用时不能更改。
- 已核对方向键、Home、Enter、Escape、Tab、点击外部关闭、焦点返回及增强关闭后的禁用状态。
- 浏览器检查未发现 error / warn；1.2 倍缩放及 360 × 800 窗口内的菜单边界正常。
- `cargo test -p codex-plus-core --test custom_layout_runtime custom_layout_renderer_integration_preserves_existing_enhancements` 通过，包含新增开关与下拉回归测试。
- `cargo test -p codex-plus-core --test cdp_bridge injection_script_menu_` 的两项测试通过。
- 分片与组装产物一致，`git diff --check` 通过。

无未解决的 P0 / P1 / P2 问题。预览中的主题值来自参考与固定宿主令牌；运行中的应用集成效果需在重新构建启动器后核对。

---

## 既有检查记录（管理器导航与标题）

# 管理器导航与标题视觉检查

当前最终行为：应用标题横跨整个窗口，单行纯色 Codex++，高度 40px。最左栏上方切换 Agent，下方为推荐、缓存清理、关于、设置及语言/主题按钮。通用页面占满整个右侧，Agent 工作区显示对应导航。页面始终直接切换，没有页面过渡，也没有过渡模式选择项。

## 参考与最终证据

- 原始参考：`/var/folders/0s/y8fsqjhd20d_twbcc2j7mj040000gn/T/codex-clipboard-a0d0279b-decf-42ab-bc71-46275d8ce72d.png`，1652 × 1430；含桌面、标注与局部管理器。仅将左侧窄栏形态作为参考，后续用户指令覆盖标题、通用页与动画要求。
- 最终截图：`/tmp/codex-agent-sidebar/instant-only-final.jpg`，1200 × 850，CSS 视口 1200 × 850，DPR 1，中文 / 深色 / 设置。
- 修改前设置截图：`/tmp/codex-agent-sidebar/page-transition-options.jpg`，1200 × 850，DPR 1，中文 / 深色 / 设置。
- 窄窗历史检查：`/tmp/codex-agent-sidebar/compact-header-light.jpg`，960 × 720；应用栏滚动检查：`/tmp/codex-agent-sidebar/global-rail-short.jpg`，960 × 480。
- 当前预览：`http://127.0.0.1:1420/node_modules/.cache/agent-sidebar-preview.html`，使用隔离 Tauri 演示响应，不读取或写入本机配置。

## 比较结果

将修改前设置截图与最终截图放在同一比较输入中，视口、主题、页面和数据相同。页面切换效果行已移除，后续卡片自然上移；应用标题、导航、现有设置排版和按钮样式保持一致，没有出现空行或错位。

- 字体与文字：Codex++ 为纯色主题前景，副标题已移除。页面与设置沿用原有字体、字号及层级。
- 布局与间距：顶部 40px；最左栏 64px。Agent 工作区为 64px / 232px / 内容，窄窗导航收至 68px；通用页为 64px / 内容，没有中间空列。
- 色彩：深浅主题保持可读，通用入口选中状态与键盘焦点可区分；标题没有彩虹或光晕。
- 图像：Codex / Grok 使用随应用打包的真实品牌 SVG；来源和许可位于 assets/agents/LICENSE.txt，图标不依赖运行时网络加载。
- 内容：保留用户指定通用入口、语言与主题控制；设置页不再提供平滑/无动画的选择。

## 交互与检查

最终在浏览器逐次访问设置、关于和 Agent 概览，标题与正文的 animation、transition、transform 均为 none；页面过渡数据属性不存在，设置选择元素数量为 0。通用页隐藏中间导航，点击当前 Codex 能正常返回概览并恢复导航。浏览器控制台没有错误。

此前已验证四个通用入口、Codex/Grok 返回行为、短窗口滚动、深浅主题和保存失败回滚。动画移除仅清理页面过渡，不修改导航数据刷新或引入点击延迟。旧偏好存储值不再读取，因此不会重新开启动画。

`npm run check`、`npm run vite:build`、`git diff --check` 通过。原生后端持久化不属于本次改动，未重新测试。全仓 i18n 校验存在此前已有的缺项/陈旧键；本次已移除不再使用的四条过渡选项翻译。

## 修改与复查记录

1. 初版图标被全局图片宽度约束缩至 26px，调整按钮内边距后恢复 28 × 28px。
2. 用户指出标题属于整个窗口：移至全宽标题行，并清理窄窗隐藏规则与工作区固定高度。
3. 通用入口移至应用栏，语言/主题也移入。缩小栏内横向内边距，滚动条出现时按钮仍完整。
4. 用户要求通用页面替换整个右侧：条件移除 Agent 导航，修正两列网格特异性，支持点击已选 Agent 返回。
5. 标题副标题与彩虹移除，顶栏由 60px 缩至 40px。
6. 曾提供平滑与无动画对比，平滑时长由 220ms 调至 400ms。用户最终选择全部移除页面过渡：动画关键帧、偏好状态与存储逻辑、模式选择、相关翻译和演示版本参数逻辑均已清理；最终截图与浏览器样式检查通过。

没有剩余 P0 / P1 / P2 发现。

final result: passed
