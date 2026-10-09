const repository = "BigPizzaV3/CodexPlusPlus";
const releasesUrl = `https://github.com/${repository}/releases/latest`;

const translations = {
  "跳到主要内容": "Skip to main content",
  "Codex++ 首页": "Codex++ home",
  "打开导航": "Open navigation",
  "关闭导航": "Close navigation",
  "主导航": "Main navigation",
  "能力": "Features",
  "界面": "Interface",
  "下载": "Download",
  "更新日志": "Changelog",
  "常见问题": "FAQ",
  "语言选择": "Language",
  "Codex++ 图标": "Codex++ icon",
  "开源 · Windows / macOS": "Open source · Windows / macOS",
  "让 Codex 更顺手。": "Make Codex easier to use.",
  "给 Codex 桌面应用加一点「++」。统一管理供应商与模型，让会话更好用，也让工作区更像你。": "Add a little ++ to the Codex desktop app. Manage providers and models in one place, make conversations easier to use, and shape your workspace around you.",
  "下载最新版": "Download latest",
  "下载 Windows 版": "Download for Windows",
  "下载 macOS 版": "Download for macOS",
  "查看源码": "View source",
  "项目状态": "Project status",
  "最新版": "Latest",
  "GitHub 最新发布": "Latest GitHub release",
  "查看": "View",
  "继续查看": "Continue",
  "继续了解": "Explore more",
  "产品概览": "Product overview",
  "4 种": "4 modes",
  "供应商模式": "Provider modes",
  "按模型": "Per model",
  "上下文与压缩阈值": "Context and compaction limits",
  "可插拔": "Pluggable",
  "拓展与个性化": "Extensions and personalization",
  "零补丁": "Zero patches",
  "不修改 app.asar": "Leaves app.asar untouched",
  "一个管理工具，覆盖完整工作流": "One manager for the complete workflow",
  "从模型配置到日常体验": "From model settings to everyday work",
  "Codex++ 通过 CDP 与本地辅助服务提供供应商切换、协议转换、会话管理和界面增强，每项增强都可以按需开关。": "Codex++ uses CDP and a local helper to provide provider switching, protocol conversion, session management, and interface enhancements. Turn each enhancement on or off as needed.",
  "供应商自由切换": "Switch providers freely",
  "官方登录、官方登录 + API、纯 API 与聚合供应商，选择适合自己的模型请求方式。": "Choose how to route model requests: official sign-in, official sign-in + API, API-only, or aggregate providers.",
  "模型测试与 Provider Doctor": "Model tests and Provider Doctor",
  "每个模型，各有空间": "Give every model its own space",
  "按模型设置上下文窗口与自动压缩阈值，支持模型元数据导入，未指定后缀时沿用供应商级配置。": "Set context windows and automatic compaction limits per model, and import model metadata. Models without a suffix keep the provider-level settings.",
  "模型窗口后缀，例如 1M / 200K": "Model window suffixes, such as 1M / 200K",
  "按供应商选择 MCP、Skill 和 Plugin": "Select MCP servers, skills, and plugins per provider",
  "会话管理更省心": "Simpler session management",
  "本地会话扫描、删除与撤销、批量管理、Markdown 导出和 Token 历史，整理会话更方便。": "Organize conversations with local session scanning, deletion and undo, batch management, Markdown export, and token history.",
  "项目移动与线程 ID": "Project moves and thread IDs",
  "会话宽度与滚动位置恢复": "Conversation width and scroll position recovery",
  "拓展你的工作区": "Extend your workspace",
  "浏览与管理社区用户脚本，按需添加模型选择、用量统计等能力，也可以编写自己的脚本。": "Browse and manage community user scripts to add model selectors, usage statistics, and more. You can also write your own scripts.",
  "拓展市场与启停管理": "Extension marketplace and activation controls",
  "MCP、Skill 与 Codex 插件管理": "Manage MCP servers, skills, and Codex plugins",
  "输入与阅读，都顺手": "Smoother writing and reading",
  "语音输入、粘贴修复、下一步建议与回答大纲，让输入和阅读更流畅。": "Dictation, paste fixes, next-step suggestions, and answer outlines make writing and reading easier.",
  "彩虹粒子、烟花与星光打字特效": "Rainbow particles, fireworks, and starlight typing effects",
  "单项开关与增强总开关": "Individual controls and a master enhancement switch",
  "用量可见，也有陪伴": "Track usage with a desktop companion",
  "用量挂件、可更换的桌宠角色、任务状态和预算提醒，搭配皮肤与自定义图片。": "Usage widgets, interchangeable desktop pets, task status, and budget alerts, paired with themes and custom images.",
  "开发预览 · 默认关闭": "Development preview · Off by default",
  "拖动、缩放与更换角色图片": "Drag, resize, and change character images",
  "微信连接支持扫码连接本机 Codex，每个联系人映射到独立会话，并可配置允许访问的微信用户。": "Connect WeChat to local Codex by scanning a QR code. Each contact gets a separate session, with a configurable list of allowed WeChat users.",
  "Codex++ 管理工具真实界面": "The real Codex++ manager interface",
  "把常用增强放到手边": "Keep everyday enhancements within reach",
  "在管理工具中集中配置，在 Codex 内直接使用。模型列表、会话操作、输入行为和阅读布局，都可以按自己的习惯开关。": "Configure everything in the manager and use it directly in Codex. Adjust model lists, session actions, input behavior, and reading layouts to your habits.",
  "Codex++ 管理工具：集中管理模型、会话、输入体验和挂件设置": "Codex++ Manager: manage models, sessions, input behavior, and widget settings in one place",
  "Codex++ 管理工具": "Codex++ Manager",
  "从供应商配置到界面增强，一个管理工具就够了": "One manager for provider settings and interface enhancements",
  "Codex 内的增强设置：后端状态、模型列表、Fast 按钮与会话操作": "Enhancement settings in Codex: backend status, model list, Fast button, and session actions",
  "Codex 内的增强设置": "Enhancement settings in Codex",
  "常用设置就在工作区里": "Everyday settings right in your workspace",
  "Codex++ 拓展市场：浏览社区脚本和工作流工具": "Codex++ extension marketplace: browse community scripts and workflow tools",
  "社区拓展市场": "Community extension marketplace",
  "发现、安装和管理工作流工具": "Discover, install, and manage workflow tools",
  "这里的「拓展」是 Codex++ 用户脚本，与 Codex 官方插件市场是两个入口。社区脚本的服务配置和使用要求以各自说明为准。": "Extensions here are Codex++ user scripts, accessed separately from the official Codex plugin marketplace. Follow each community script's documentation for service setup and usage requirements.",
  "用户脚本开发说明 ↗": "User script development guide ↗",
  "角色用量挂件：气泡显示 Token 统计和运行状态": "Character usage widget: bubbles show token statistics and task status",
  "用量挂件与桌宠 · 开发预览": "Usage widgets and desktop pets · Development preview",
  "查看本地会话 Token 与任务状态": "View local session tokens and task status",
  "Codex 输入框中的彩虹粒子打字特效": "Rainbow particle typing effects in the Codex input box",
  "光标打字特效": "Cursor typing effects",
  "可随时关闭，系统减少动态效果时暂停": "Turn them off anytime; paused when reduced motion is enabled",
  "挂件与桌宠截图来自开发版本，正式发布后的开启方式与统计口径以版本说明为准。": "Widget and desktop pet screenshots are from a development build. See the release notes for activation instructions and how usage is measured in the released version.",
  "用量挂件说明 ↗": "Usage widget guide ↗",
  "原始功能、鲸鱼角色及内置素材来自": "Original features, whale character, and bundled assets come from",
  "。": ".",
  "第一次使用": "Getting started",
  "三步进入你的 Codex": "Start using Codex in three steps",
  "安装": "Install",
  "先安装官方 Codex / ChatGPT 桌面应用，再下载 Windows 安装程序或 macOS 通用 DMG。": "Install the official Codex / ChatGPT desktop app first, then download the Windows installer or universal macOS DMG.",
  "配置": "Configure",
  "打开管理工具，确认 Codex 路径，再设置供应商和需要的增强。": "Open the manager, confirm the Codex path, then configure your provider and desired enhancements.",
  "启动": "Launch",
  "以后从 Codex++ 入口启动，已保存的配置会自动加载。依赖注入脚本的设置通常需要保存后重启。": "Launch from the Codex++ entry point to load your saved configuration. Settings that depend on injected scripts usually require saving and restarting.",
  "供应商与模型": "Providers and models",
  "选一种适合你的使用方式": "Choose the setup that suits you",
  "官方登录": "Official sign-in",
  "使用 ChatGPT / Codex 官方账号。": "Use your official ChatGPT / Codex account.",
  "官方登录 + API": "Official sign-in + API",
  "保留官方登录状态与插件入口，模型请求始终走配置的兼容 API，不消耗官方额度。": "Keep official sign-in and plugin access while routing all model requests through your configured compatible API, without using your official quota.",
  "纯 API": "API-only",
  "使用自定义 Base URL / Key，无需官方账号。": "Use a custom Base URL and key without an official account.",
  "聚合供应商": "Aggregate providers",
  "在多个 API 供应商之间故障转移，或按会话、请求、权重轮转。": "Route across multiple API providers with failover or conversation, request, or weighted round-robin.",
  "官方登录 + API 不会先用官方额度再切 API。Chat Completions 供应商可通过本地代理转换为 Codex 使用的 Responses 协议。": "Official sign-in + API does not use official quota before switching to the API. A local proxy can convert Chat Completions providers to the Responses protocol used by Codex.",
  "给每个模型设置自己的上下文窗口": "Set a context window for each model",
  "在模型列表中使用窗口后缀，支持 1M、200K 或纯数字。请按实际供应商能力填写；未指定后缀的模型继续使用供应商级上下文配置。": "Use window suffixes in the model list, such as 1M, 200K, or plain integers. Match your provider's actual capabilities; models without a suffix keep the provider-level context settings.",
  "模型列表窗口后缀示例": "Example model list with context window suffixes",
  "下载 Codex++": "Download Codex++",
  "选择你的平台": "Choose your platform",
  "当前稳定版": "Current stable release",
  "。安装包由 GitHub Actions 从公开源码自动构建。": ". Installers are built automatically from public source by GitHub Actions.",
  "选择操作系统": "Choose an operating system",
  "Windows 安装程序": "Windows installer",
  "包含 Codex++ 启动器和管理工具，并创建桌面与开始菜单入口。": "Includes the Codex++ launcher and manager, with desktop and Start menu shortcuts.",
  "EXE · 正在读取文件信息": "EXE · Loading file details",
  "DMG · 正在读取文件信息": "DMG · Loading file details",
  "下载安装程序": "Download installer",
  "macOS 通用安装包": "Universal macOS installer",
  "一个 DMG 同时支持 Apple Silicon 与 Intel Mac，安装到 Applications 即可。": "One DMG supports both Apple Silicon and Intel Macs. Install it in Applications.",
  "下载 macOS 通用版": "Download universal macOS build",
  "通用安装包": "Universal installer",
  "前往 Releases 查看安装包": "View installers on Releases",
  "前往 Releases 选择对应架构": "Choose your architecture on Releases",
  "查看 Release Notes": "View release notes",
  "历史版本": "Previous releases",
  "透明边界": "Clear boundaries",
  "增强，但不接管": "Enhance without taking over",
  "不修改原始安装": "Leaves the original installation intact",
  "不修改 Codex 的": "Does not modify Codex's",
  "，不向官方应用安装目录写入补丁文件。": " or write patch files into the official app directory.",
  "密钥留在本机": "Keys stay on your device",
  "供应商密钥保存在本地配置边界中，不上传到 Codex++ 项目或广告服务。": "Provider keys stay within local configuration and are never uploaded to the Codex++ project or advertising services.",
  "功能可以关闭": "Features can be disabled",
  "界面增强既有单项开关，也有总开关；关闭后仍可只使用供应商与启动管理。": "Interface enhancements have both individual controls and a master switch. With them off, provider and launch management remain available.",
  "公开构建过程": "Public build process",
  "源码、Issue、发布记录和安装包构建工作流都可以在 GitHub 查阅。": "Source, issues, release history, and installer workflows are all available on GitHub.",
  "安装前可能想知道": "What to know before installing",
  "Codex++ 是 OpenAI 官方产品吗？": "Is Codex++ an official OpenAI product?",
  "不是。Codex++ 是社区维护的开源外部启动器与管理工具，面向 OpenAI Codex / ChatGPT 桌面应用使用。": "No. Codex++ is a community-maintained, open-source external launcher and manager for the OpenAI Codex / ChatGPT desktop app.",
  "安装后为什么有两个入口？": "Why are there two app entries after installation?",
  "“Codex++”用于日常静默启动并加载配置；“Codex++ 管理工具”用于管理供应商、增强、脚本、更新和诊断。": "Codex++ launches silently for everyday use and loads your configuration. Codex++ Manager configures providers, enhancements, scripts, updates, and diagnostics.",
  "不使用第三方 API 可以吗？": "Can I use it without a third-party API?",
  "可以。选择官方登录模式即可只使用 ChatGPT / Codex 官方账号，Codex++ 也能仅作为启动和增强管理工具。": "Yes. Select official sign-in to use only your ChatGPT / Codex account. Codex++ can also serve solely as a launcher and enhancement manager.",
  "启动后，为什么没有 Codex++ 菜单？": "Why is the Codex++ menu missing after launch?",
  "确认从 Codex++ 入口启动。在管理工具的「安装维护」与「关于」页面检查应用路径、启动状态和诊断日志。": "Launch from the Codex++ entry point. Check the app path, launch status, and diagnostic logs in the manager's Maintenance and About pages.",
  "切换供应商后，请求为什么失败？": "Why do requests fail after switching providers?",
  "先在供应商详情中运行模型测试或 Provider Doctor，确认协议、Base URL、Key 与模型匹配。提交反馈前请隐藏密钥与认证信息。": "Run a model test or Provider Doctor in the provider details, and verify that the protocol, Base URL, key, and model match. Hide keys and authentication details before submitting feedback.",
  "增强功能可以关闭吗？官方应用更新后还能用吗？": "Can I disable enhancements? Will they work after official app updates?",
  "可以分别关闭，也可以关闭增强总开关。Codex++ 依赖官方应用的页面结构、CDP 和本地数据格式；官方更新后，部分功能可能需要跟随适配。修改供应商配置或会话数据前，请保留备份。": "Disable enhancements individually or with the master switch. Codex++ depends on the official app's page structure, CDP, and local data formats, so some features may need updates after an official release. Keep backups before changing provider settings or session data.",
  "交流与支持": "Community and support",
  "一起把「++」做得更好": "Make ++ better together",
  "欢迎反馈问题、分享拓展、主题和使用经验。提交 Issue 时，请附上系统、Codex++ 版本、复现步骤与已脱敏的日志。": "Report issues and share extensions, themes, and tips. Include your operating system, Codex++ version, steps to reproduce, and redacted logs when opening an issue.",
  "QQ 交流 4 群 · 1127858981 ↗": "QQ community group 4 · 1127858981 ↗",
  "感谢赞助商": "Thanks to our sponsors",
  "感谢以下赞助商对项目的支持。服务范围、价格与活动以各平台官网说明为准。": "Thanks to these sponsors for supporting the project. Refer to each provider's official website for services, prices, and offers.",
  "赞助商列表自动更新 · 查看来源 ↗": "Sponsors update automatically · View source ↗",
  "赞助商列表加载中…": "Loading sponsors…",
  "赞助商列表暂时无法加载，请稍后重试或查看来源。": "Sponsors could not be loaded. Try again later or view the source.",
  "暂无有效赞助商。": "No active sponsors.",
  "重新加载": "Retry",
  "请启用 JavaScript 加载最新赞助商列表，或通过上方链接查看来源。": "Enable JavaScript to load the latest sponsors, or use the source link above.",
  "想支持项目或展示品牌？": "Want to support the project or showcase your brand?",
  "联系维护者 ↗": "Contact the maintainer ↗",
  "给 Codex 桌面应用加一点「++」": "Add a little ++ to the Codex desktop app",
  "从供应商配置到界面增强，先从管理工具开始。": "From provider settings to interface enhancements, start with the manager.",
  "反馈问题": "Report an issue",
  "社区维护的 Codex 桌面增强与管理工具。": "A community-maintained desktop enhancement and management tool for Codex.",
  "版本列表": "Release list",
  "Codex++ 历史 Releases": "Previous Codex++ releases",
  "v1.2.50 及更早": "v1.2.50 and earlier"
};

const pageMetadata = {
  zh: {
    title: "Codex++ - Codex 桌面增强与管理工具",
    description: "Codex++ 是面向 OpenAI Codex / ChatGPT 桌面应用的开源启动器与管理工具，统一管理供应商与模型，提供会话管理、社区拓展、语音输入与界面个性化。",
    ogTitle: "Codex++ - 让 Codex 更顺手",
    ogDescription: "给 Codex 桌面应用加一点 ++。供应商与模型、工作流增强、拓展与个性化，适用于 Windows 与 macOS。",
  },
  en: {
    title: "Codex++ - Desktop enhancements and management for Codex",
    description: "Codex++ is an open-source launcher and manager for the OpenAI Codex / ChatGPT desktop app. Manage providers and models, organize sessions, explore community extensions, and personalize input and interface behavior.",
    ogTitle: "Codex++ - Make Codex easier to use",
    ogDescription: "Add a little ++ to the Codex desktop app. Providers and models, workflow enhancements, extensions, and personalization for Windows and macOS.",
  },
  changelog: {
    zh: {
      title: "Codex++ 更新日志",
      description: "Codex++ 更新日志，查看每个版本的功能、改进与修复。",
      ogTitle: "Codex++ 更新日志",
      ogDescription: "查看 Codex++ 每个版本的功能、改进与修复。",
    },
    en: {
      title: "Codex++ Changelog",
      description: "Codex++ changelog with release features, improvements, and fixes.",
      ogTitle: "Codex++ Changelog",
      ogDescription: "Explore the features, improvements, and fixes in every Codex++ release.",
    },
  },
};

const translatedTextNodes = new Map();
const translatedAttributes = new Map();
let currentLanguage = document.documentElement.dataset.language === "en" ? "en" : "zh";
let detectedPlatform = "windows";
let releaseAssets = new Map();
let releaseVersion = null;
let repositoryStars = null;
let sponsorState = "loading";
let sponsorCount = 0;
let sponsorsLoading = false;

const translate = (text) => currentLanguage === "en" ? (translations[text] || text) : text;

const rememberTranslatableContent = () => {
  const walker = document.createTreeWalker(document.body, NodeFilter.SHOW_TEXT);
  let node;
  while ((node = walker.nextNode())) {
    const source = node.nodeValue.trim();
    if (source && translations[source]) translatedTextNodes.set(node, source);
  }

  document.querySelectorAll("[aria-label], [alt]").forEach((element) => {
    ["aria-label", "alt"].forEach((attribute) => {
      const source = element.getAttribute(attribute);
      if (source && translations[source]) translatedAttributes.set(`${translatedAttributes.size}`, { element, attribute, source });
    });
  });
};

const applyLanguage = (language) => {
  currentLanguage = language === "en" ? "en" : "zh";
  document.documentElement.lang = currentLanguage === "zh" ? "zh-CN" : "en";
  document.documentElement.dataset.language = currentLanguage;

  translatedTextNodes.forEach((source, node) => {
    const leading = node.nodeValue.match(/^\s*/)?.[0] || "";
    const trailing = node.nodeValue.match(/\s*$/)?.[0] || "";
    node.nodeValue = `${leading}${translate(source)}${trailing}`;
  });
  translatedAttributes.forEach(({ element, attribute, source }) => {
    element.setAttribute(attribute, translate(source));
  });

  const metadata = document.body.classList.contains("changelog-page")
    ? pageMetadata.changelog[currentLanguage]
    : pageMetadata[currentLanguage];
  document.title = metadata.title;
  document.querySelector('meta[name="description"]')?.setAttribute("content", metadata.description);
  document.querySelector('meta[property="og:title"]')?.setAttribute("content", metadata.ogTitle);
  document.querySelector('meta[property="og:description"]')?.setAttribute("content", metadata.ogDescription);

  document.querySelectorAll(".language-switch [data-language]").forEach((button) => {
    button.setAttribute("aria-pressed", String(button.dataset.language === currentLanguage));
  });

  const menuButton = document.querySelector("[data-menu-button]");
  if (menuButton) {
    const open = menuButton.getAttribute("aria-expanded") === "true";
    menuButton.setAttribute("aria-label", translate(open ? "关闭导航" : "打开导航"));
  }

  updateDynamicLabels();
  try {
    localStorage.setItem("codex-plus-language", currentLanguage);
  } catch {
    // The page remains usable when storage is disabled.
  }
};

const updateDynamicLabels = () => {
  const heroLabel = document.querySelector("[data-hero-download-label]");
  if (heroLabel) {
    heroLabel.textContent = translate(detectedPlatform === "windows" ? "下载 Windows 版" : "下载 macOS 版");
  }

  document.querySelectorAll("[data-version]").forEach((element) => {
    element.textContent = releaseVersion || translate("GitHub 最新发布");
  });
  releaseAssets.forEach(({ asset, legacyMacBuilds }, platform) => setPlatformAsset(platform, asset, legacyMacBuilds));
  updateSponsorStatus();

  if (Number.isFinite(repositoryStars)) {
    const compact = new Intl.NumberFormat(currentLanguage === "zh" ? "zh-CN" : "en-US", {
      notation: "compact",
      maximumFractionDigits: 1,
    }).format(repositoryStars);
    document.querySelectorAll("[data-stars]").forEach((element) => {
      element.textContent = compact;
    });
  }
};

const platformConfig = {
  windows: {
    assetPattern: /windows-x64-setup\.exe$/i,
    type: "EXE",
  },
  macos: {
    assetPattern: /macos-universal\.dmg$/i,
    type: "DMG",
  },
};

const formatBytes = (bytes) => {
  if (!Number.isFinite(bytes) || bytes <= 0) return "";
  const megabytes = bytes / (1024 * 1024);
  return `${megabytes.toFixed(megabytes >= 100 ? 0 : 1)} MB`;
};

const setPlatformAsset = (platform, asset, legacyMacBuilds = false) => {
  const config = platformConfig[platform];
  const link = document.querySelector(`[data-download="${platform}"]`);
  const meta = document.querySelector(`[data-asset-meta="${platform}"]`);
  if (!config || !link || !meta) return;

  if (asset) {
    link.href = asset.browser_download_url;
    const size = formatBytes(asset.size);
    const architecture = platform === "macos" ? ` · ${translate("通用安装包")}` : "";
    meta.textContent = `${config.type}${architecture}${size ? ` · ${size}` : ""} · GitHub Release`;
    return;
  }

  link.href = releasesUrl;
  meta.textContent = translate(legacyMacBuilds ? "前往 Releases 选择对应架构" : "前往 Releases 查看安装包");
};

const updateRelease = async () => {
  try {
    const response = await fetch(`https://api.github.com/repos/${repository}/releases/latest`, {
      headers: { Accept: "application/vnd.github+json" },
    });
    if (!response.ok) throw new Error(`GitHub release API returned ${response.status}`);

    const release = await response.json();
    releaseVersion = typeof release.tag_name === "string" ? release.tag_name.trim() || null : null;
    const assets = Array.isArray(release.assets) ? release.assets : [];
    const downloadableAssets = assets.filter((asset) => {
      if (typeof asset.name !== "string" || typeof asset.browser_download_url !== "string") return false;
      try {
        const url = new URL(asset.browser_download_url);
        return url.protocol === "https:" && url.hostname === "github.com" &&
          url.pathname.startsWith(`/${repository}/releases/download/`);
      } catch {
        return false;
      }
    });
    const legacyMacBuilds = downloadableAssets.some((asset) => /macos-(arm64|x64)\.dmg$/i.test(asset.name));

    Object.entries(platformConfig).forEach(([platform, config]) => {
      const asset = downloadableAssets.find((item) => config.assetPattern.test(item.name));
      releaseAssets.set(platform, { asset, legacyMacBuilds: platform === "macos" && legacyMacBuilds });
    });
    updateDynamicLabels();
  } catch (error) {
    releaseVersion = null;
    Object.keys(platformConfig).forEach((platform) => releaseAssets.set(platform, { asset: null }));
    updateDynamicLabels();
    console.info("Release details unavailable; using GitHub Releases", error);
  }
};

const updateRepositoryStats = async () => {
  try {
    const response = await fetch(`https://api.github.com/repos/${repository}`, {
      headers: { Accept: "application/vnd.github+json" },
    });
    if (!response.ok) return;
    const repositoryData = await response.json();
    const stars = Number(repositoryData.stargazers_count);
    if (!Number.isFinite(stars)) return;

    repositoryStars = stars;
    updateDynamicLabels();
  } catch (error) {
    console.info("Using bundled repository stats", error);
  }
};

// 与管理工具共用远端清单，不保留需要单独维护的品牌名单或图片映射。
const sponsorSources = [
  "https://raw.githubusercontent.com/BigPizzaV3/Ad-List/main/ads.json",
  "https://cdn.jsdelivr.net/gh/BigPizzaV3/Ad-List@main/ads.json",
];

const sponsorUrl = (value, sourceUrl) => {
  if (typeof value !== "string" || !value.trim()) return "";
  try {
    const url = new URL(value.trim(), sourceUrl);
    return ["https:", "http:"].includes(url.protocol) ? url.href : "";
  } catch {
    return "";
  }
};

const normalizeSponsors = (payload, sourceUrl, now = Date.now()) => {
  const normalize = (ad) => {
    if (!ad || ad.type !== "sponsor") return null;
    if (![ad.title, ad.description, ad.url].every((value) => typeof value === "string" && value.trim())) return null;
    const url = sponsorUrl(ad.url);
    const expiresAt = typeof ad.expires_at === "string" ? Date.parse(ad.expires_at) : NaN;
    if (!url || (Number.isFinite(expiresAt) && expiresAt < now)) return null;
    return {
      title: ad.title.trim(),
      description: ad.description.trim(),
      url,
      image: sponsorUrl(ad.image, sourceUrl),
      highlights: Array.isArray(ad.highlights)
        ? ad.highlights.filter((value) => typeof value === "string" && value.trim()).map((value) => value.trim())
        : [],
    };
  };
  const seen = new Set();
  const collect = (items) => items.map(normalize).filter((ad) => {
    if (!ad) return false;
    const url = new URL(ad.url);
    const identity = `${url.host}${url.pathname}`.replace(/\/+$/, "").toLowerCase();
    if (seen.has(identity)) return false;
    seen.add(identity);
    return true;
  });
  const rawFeatured = payload?.top_ad ?? (Array.isArray(payload?.topAds) && payload.topAds.length ? payload.topAds : payload?.topAd);
  const featured = collect(Array.isArray(rawFeatured) ? rawFeatured : rawFeatured ? [rawFeatured] : []);
  const ads = collect(Array.isArray(payload?.ads) ? payload.ads : []);
  return { featured, ads };
};

const fetchSponsorList = async () => {
  const cacheBust = Date.now();
  let lastError;
  for (const sourceUrl of sponsorSources) {
    const controller = new AbortController();
    const timer = setTimeout(() => controller.abort(), 8000);
    try {
      const response = await fetch(`${sourceUrl}?v=${cacheBust}`, {
        cache: "no-store",
        credentials: "omit",
        signal: controller.signal,
      });
      if (!response.ok) throw new Error(`Sponsor source returned ${response.status}`);
      const payload = await response.json();
      if (!payload || typeof payload !== "object" || Array.isArray(payload) ||
          (!Array.isArray(payload.ads) && ![payload.top_ad, payload.topAds, payload.topAd].some((value) => value && typeof value === "object"))) {
        throw new Error("Invalid sponsor list");
      }
      return { payload, sourceUrl };
    } catch (error) {
      lastError = error;
    } finally {
      clearTimeout(timer);
    }
  }
  throw lastError || new Error("Sponsor list unavailable");
};

const updateSponsorStatus = () => {
  const status = document.querySelector("[data-sponsor-status]");
  if (!status) return;
  const message = sponsorState === "loading" ? "赞助商列表加载中…"
    : sponsorState === "error" ? "赞助商列表暂时无法加载，请稍后重试或查看来源。"
    : sponsorCount === 0 ? "暂无有效赞助商。" : "";
  status.textContent = translate(message);
  status.hidden = !message;
  const retry = document.querySelector("[data-sponsor-retry]");
  if (retry) retry.hidden = sponsorState !== "error";
};

const createSponsorCard = (ad, featured) => {
  const card = document.createElement("a");
  card.className = featured ? "sponsor-banner" : "sponsor-card";
  card.href = ad.url;
  card.target = "_blank";
  card.rel = "noreferrer";
  const placeholder = document.createElement("span");
  placeholder.className = "sponsor-mark";
  placeholder.textContent = ad.title.slice(0, 1);
  placeholder.setAttribute("aria-hidden", "true");
  if (ad.image) {
    const image = document.createElement("img");
    image.src = ad.image;
    image.alt = "";
    image.loading = "lazy";
    image.addEventListener("error", () => image.replaceWith(placeholder), { once: true });
    card.append(image);
  } else {
    card.append(placeholder);
  }
  const copy = document.createElement("div");
  copy.className = "sponsor-copy";
  const title = document.createElement("h4");
  title.textContent = `${ad.title} ↗`;
  const description = document.createElement("p");
  description.textContent = ad.description;
  copy.append(title, description);
  if (ad.highlights.length) {
    const highlights = document.createElement("ul");
    highlights.className = "sponsor-highlights";
    ad.highlights.forEach((value) => {
      const item = document.createElement("li");
      item.textContent = value;
      highlights.append(item);
    });
    copy.append(highlights);
  }
  card.append(copy);
  return card;
};

const updateSponsors = async () => {
  const featured = document.querySelector("[data-sponsor-featured]");
  const grid = document.querySelector("[data-sponsor-grid]");
  if (!featured || !grid || sponsorsLoading) return;
  sponsorsLoading = true;
  sponsorState = "loading";
  updateSponsorStatus();
  try {
    const { payload, sourceUrl } = await fetchSponsorList();
    const sponsors = normalizeSponsors(payload, sourceUrl);
    featured.replaceChildren(...sponsors.featured.map((ad) => createSponsorCard(ad, true)));
    grid.replaceChildren(...sponsors.ads.map((ad) => createSponsorCard(ad, false)));
    sponsorCount = sponsors.featured.length + sponsors.ads.length;
    sponsorState = "loaded";
  } catch (error) {
    featured.replaceChildren();
    grid.replaceChildren();
    sponsorCount = 0;
    sponsorState = "error";
    console.info("Sponsor list unavailable", error);
  } finally {
    sponsorsLoading = false;
    updateSponsorStatus();
  }
};

const setupSponsors = () => {
  document.querySelector("[data-sponsor-retry]")?.addEventListener("click", updateSponsors);
  updateSponsors();
};

const platformFromDevice = () => {
  const platform = navigator.userAgentData?.platform || navigator.platform || navigator.userAgent;
  if (/mac/i.test(platform)) return "macos";
  return "windows";
};

const selectPlatform = (platform, focusTab = false) => {
  const tabs = [...document.querySelectorAll("[data-platform]")];
  const panels = [...document.querySelectorAll("[data-platform-panel]")];
  const selectedTab = tabs.find((tab) => tab.dataset.platform === platform);
  if (!selectedTab) return;

  tabs.forEach((tab) => {
    const selected = tab === selectedTab;
    tab.setAttribute("aria-selected", String(selected));
    tab.tabIndex = selected ? 0 : -1;
  });

  panels.forEach((panel) => {
    panel.hidden = panel.dataset.platformPanel !== platform;
  });

  if (focusTab) selectedTab.focus();
};

const setupPlatformPicker = () => {
  const tabs = [...document.querySelectorAll("[data-platform]")];
  detectedPlatform = platformFromDevice();
  selectPlatform(detectedPlatform);

  const heroDownload = document.querySelector("[data-hero-download]");
  const heroLabel = document.querySelector("[data-hero-download-label]");
  if (heroLabel) updateDynamicLabels();
  if (heroDownload) {
    heroDownload.addEventListener("click", () => selectPlatform(detectedPlatform));
  }

  tabs.forEach((tab, index) => {
    tab.addEventListener("click", () => selectPlatform(tab.dataset.platform));
    tab.addEventListener("keydown", (event) => {
      if (!['ArrowLeft', 'ArrowRight'].includes(event.key)) return;
      event.preventDefault();
      const direction = event.key === "ArrowRight" ? 1 : -1;
      const nextIndex = (index + direction + tabs.length) % tabs.length;
      selectPlatform(tabs[nextIndex].dataset.platform, true);
    });
  });
};

const setupNavigation = () => {
  const header = document.querySelector("[data-header]");
  const menuButton = document.querySelector("[data-menu-button]");
  const navigation = document.querySelector("[data-nav]");

  const updateHeader = () => header?.classList.toggle("is-scrolled", window.scrollY > 24);
  updateHeader();
  window.addEventListener("scroll", updateHeader, { passive: true });

  const closeMenu = () => {
    menuButton?.setAttribute("aria-expanded", "false");
    menuButton?.setAttribute("aria-label", translate("打开导航"));
    navigation?.classList.remove("is-open");
    document.body.classList.remove("menu-open");
  };

  menuButton?.addEventListener("click", () => {
    const open = menuButton.getAttribute("aria-expanded") !== "true";
    menuButton.setAttribute("aria-expanded", String(open));
    menuButton.setAttribute("aria-label", translate(open ? "关闭导航" : "打开导航"));
    navigation?.classList.toggle("is-open", open);
    document.body.classList.toggle("menu-open", open);
  });

  navigation?.querySelectorAll("a").forEach((link) => link.addEventListener("click", closeMenu));
  window.addEventListener("resize", () => {
    if (window.innerWidth > 760) closeMenu();
  });
};

const setupReveal = () => {
  const elements = [...document.querySelectorAll("[data-reveal]")];
  if (!("IntersectionObserver" in window)) {
    elements.forEach((element) => element.classList.add("is-visible"));
    return;
  }

  const observer = new IntersectionObserver(
    (entries) => {
      entries.forEach((entry) => {
        if (!entry.isIntersecting) return;
        entry.target.classList.add("is-visible");
        observer.unobserve(entry.target);
      });
    },
    { rootMargin: "0px 0px -8%", threshold: 0.08 },
  );
  elements.forEach((element) => observer.observe(element));
};

const setupLanguageSwitcher = () => {
  rememberTranslatableContent();
  document.querySelectorAll(".language-switch [data-language]").forEach((button) => {
    button.addEventListener("click", () => applyLanguage(button.dataset.language));
  });
  applyLanguage(currentLanguage);
};

const setupHeroNetwork = () => {
  const canvas = document.querySelector("[data-hero-network]");
  const hero = canvas?.closest(".hero");
  if (!(canvas instanceof HTMLCanvasElement) || !hero) return;

  const context = canvas.getContext("2d");
  if (!context) return;

  const reduceMotion = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
  const pointer = { x: 0, y: 0, active: false };
  let width = 0;
  let height = 0;
  let points = [];
  let animationFrame = 0;

  const createPoints = () => {
    const count = Math.max(34, Math.min(86, Math.round((width * height) / 14500)));
    points = Array.from({ length: count }, () => ({
      x: Math.random() * width,
      y: Math.random() * height,
      vx: (Math.random() - 0.5) * 0.22,
      vy: (Math.random() - 0.5) * 0.22,
      size: 1 + Math.random() * 1.6,
    }));
  };

  const resize = () => {
    const rect = hero.getBoundingClientRect();
    const scale = Math.min(window.devicePixelRatio || 1, 2);
    width = rect.width;
    height = rect.height;
    canvas.width = Math.round(width * scale);
    canvas.height = Math.round(height * scale);
    canvas.style.width = `${width}px`;
    canvas.style.height = `${height}px`;
    context.setTransform(scale, 0, 0, scale, 0, 0);
    createPoints();
  };

  const draw = () => {
    context.clearRect(0, 0, width, height);
    const linkDistance = width < 600 ? 108 : 138;

    points.forEach((point) => {
      if (!reduceMotion) {
        point.x += point.vx;
        point.y += point.vy;
        if (point.x < -20 || point.x > width + 20) point.vx *= -1;
        if (point.y < -20 || point.y > height + 20) point.vy *= -1;
      }
    });

    for (let index = 0; index < points.length; index += 1) {
      const point = points[index];
      for (let otherIndex = index + 1; otherIndex < points.length; otherIndex += 1) {
        const other = points[otherIndex];
        const distance = Math.hypot(point.x - other.x, point.y - other.y);
        if (distance >= linkDistance) continue;
        const pointerDistance = pointer.active
          ? Math.min(Math.hypot(pointer.x - point.x, pointer.y - point.y), Math.hypot(pointer.x - other.x, pointer.y - other.y))
          : 999;
        const highlight = Math.max(0, 1 - pointerDistance / 210);
        context.strokeStyle = `rgba(${96 + Math.round(highlight * 54)}, ${116 + Math.round(highlight * 52)}, 255, ${(1 - distance / linkDistance) * (0.22 + highlight * 0.38)})`;
        context.lineWidth = 0.7 + highlight * 0.7;
        context.beginPath();
        context.moveTo(point.x, point.y);
        context.lineTo(other.x, other.y);
        context.stroke();
      }

      const pointerDistance = pointer.active ? Math.hypot(pointer.x - point.x, pointer.y - point.y) : 999;
      const highlight = Math.max(0, 1 - pointerDistance / 190);
      context.fillStyle = `rgba(151, 164, 255, ${0.28 + highlight * 0.72})`;
      context.beginPath();
      context.arc(point.x, point.y, point.size + highlight * 2.2, 0, Math.PI * 2);
      context.fill();
    }

    if (!reduceMotion) animationFrame = requestAnimationFrame(draw);
  };

  hero.addEventListener("pointermove", (event) => {
    const rect = hero.getBoundingClientRect();
    pointer.x = event.clientX - rect.left;
    pointer.y = event.clientY - rect.top;
    pointer.active = true;
  });
  hero.addEventListener("pointerleave", () => { pointer.active = false; });
  window.addEventListener("resize", resize, { passive: true });
  window.addEventListener("pagehide", () => cancelAnimationFrame(animationFrame), { once: true });

  resize();
  draw();
};

document.querySelectorAll("[data-year]").forEach((element) => {
  element.textContent = String(new Date().getFullYear());
});

document.querySelectorAll('a[href="#downloads"]').forEach((link) => {
  if (!link.hasAttribute("data-hero-download")) link.href = "#downloads";
});

setupNavigation();
setupPlatformPicker();
setupLanguageSwitcher();
setupHeroNetwork();
setupReveal();
setupSponsors();
updateRelease();
updateRepositoryStats();

window.addEventListener("error", (event) => {
  if (event.target instanceof HTMLImageElement) {
    event.target.closest("figure")?.classList.add("image-unavailable");
  }
}, true);
