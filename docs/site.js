const repository = "BigPizzaV3/CodexPlusPlus";
const fallbackVersion = "1.2.46";

const translations = {
  "跳到主要内容": "Skip to main content",
  "Codex++ 首页": "Codex++ home",
  "打开导航": "Open navigation",
  "关闭导航": "Close navigation",
  "主导航": "Main navigation",
  "能力": "Features",
  "界面": "Interface",
  "下载": "Download",
  "常见问题": "FAQ",
  "语言选择": "Language",
  "Codex++ 图标": "Codex++ icon",
  "开源 · Windows / macOS": "Open source · Windows / macOS",
  "让 Codex 按你的方式工作。": "Make Codex work your way.",
  "一处管理供应商、模型、会话、插件、脚本和界面增强。保留 Codex 原始安装，随时开关，随时恢复。": "Manage providers, models, sessions, plugins, scripts, and interface enhancements in one place. Keep the original Codex installation intact, turn features on or off, and restore defaults at any time.",
  "下载最新版": "Download latest",
  "下载 Windows 版": "Download for Windows",
  "下载 macOS 版": "Download for macOS",
  "查看源码": "View source",
  "项目状态": "Project status",
  "最新版": "Latest",
  "继续查看": "Continue",
  "继续了解": "Explore more",
  "产品概览": "Product overview",
  "4 种": "4 modes",
  "供应商模式": "Provider modes",
  "按模型": "Per model",
  "上下文与压缩阈值": "Context and compaction limits",
  "可插拔": "Pluggable",
  "插件、脚本与皮肤": "Plugins, scripts, and themes",
  "零补丁": "Zero patches",
  "不修改 app.asar": "Leaves app.asar untouched",
  "一个管理工具，覆盖完整工作流": "One manager for the complete workflow",
  "不只是换个 API 地址": "More than switching an API endpoint",
  "Codex++ 把分散在配置文件、数据库和启动参数里的操作收拢成可视化流程，同时保留每项增强的独立开关。": "Codex++ turns operations scattered across config files, databases, and launch arguments into a visual workflow, while keeping every enhancement independently controllable.",
  "供应商与模型": "Providers and models",
  "在官方登录、混入 API、纯 API 与聚合路由之间切换，配置协议、模型、上下文窗口和自动压缩阈值。": "Switch between official sign-in, mixed API, API-only, and aggregated routing modes. Configure protocols, models, context windows, and automatic compaction thresholds.",
  "模型测试与 Provider Doctor": "Model tests and Provider Doctor",
  "会话管理": "Session management",
  "扫描本地会话，查看 Token 历史，批量整理、导出或迁移项目，不再手动翻找 SQLite 和会话目录。": "Scan local sessions, inspect token history, and organize, export, or migrate projects in batches without digging through SQLite databases and session folders.",
  "Markdown 导出与批量操作": "Markdown export and batch operations",
  "滚动位置与线程信息恢复": "Scroll position and thread recovery",
  "插件与脚本": "Plugins and scripts",
  "管理 Codex 插件、Skill、MCP Server 和用户脚本，并按供应商保存不同的工具组合。": "Manage Codex plugins, skills, MCP servers, and user scripts, with a different tool set for each provider.",
  "脚本市场与启停管理": "Script marketplace and activation controls",
  "插件入口与模型兼容处理": "Plugin entry points and model compatibility",
  "DreamSkin 主题": "DreamSkin themes",
  "浏览社区主题，在线预览、安装和更新，也可以导入本地 ZIP 包或通过链接一键换肤。": "Browse community themes, preview, install, and update them online, or import a local ZIP package and install from a link.",
  "社区主题市场": "Community theme marketplace",
  "包校验与 Safe CSS": "Package validation and Safe CSS",
  "界面增强": "Interface enhancements",
  "中文界面、会话宽度、服务层级、Goals、粘贴修复等增强按需开启，不想用的功能可以完全关闭。": "Enable localization, session width, service tiers, Goals, paste fixes, and other enhancements as needed. Anything you do not want can be fully disabled.",
  "单项开关与总开关": "Individual controls and a master switch",
  "随启动器加载，无安装目录补丁": "Loaded by the launcher without installation patches",
  "开发与诊断": "Development and diagnostics",
  "处理 Upstream worktree、Zed Remote、应用检测、环境冲突、Watcher、更新与诊断日志。": "Handle upstream worktrees, Zed Remote, app detection, environment conflicts, watchers, updates, and diagnostic logs.",
  "Windows / macOS 原生安装包": "Native Windows and macOS installers",
  "健康检查与 Release 更新": "Health checks and release updates",
  "Codex++ 管理工具真实界面": "The real Codex++ manager interface",
  "控制中心，才是 Codex++ 的主场": "The control center is where Codex++ shines",
  "供应商、增强开关、脚本和主题都在独立管理工具中完成，日常使用不用再手工修改配置文件。": "Providers, enhancement controls, scripts, and themes all live in a dedicated manager, so daily use no longer requires editing config files by hand.",
  "Codex++ 管理工具中的 DreamSkin 社区主题市场": "DreamSkin community marketplace in the Codex++ manager",
  "DreamSkin 社区主题市场": "DreamSkin community marketplace",
  "搜索、预览、安装和管理社区主题": "Search, preview, install, and manage community themes",
  "Codex++ 管理工具中的供应商配置页面": "Provider configuration in the Codex++ manager",
  "供应商配置": "Provider configuration",
  "模式、协议与切换状态集中管理": "Manage modes, protocols, and active state in one place",
  "Codex++ 管理工具中的增强设置页面": "Enhancement settings in the Codex++ manager",
  "Codex 增强": "Codex enhancements",
  "按模式选择，并精确控制每项能力": "Choose a mode and control every capability precisely",
  "Codex++ 管理工具中的脚本市场页面": "Script marketplace in the Codex++ manager",
  "脚本市场": "Script marketplace",
  "搜索、排版和本地脚本统一管理": "Search, layouts, and local script management",
  "第一次使用": "Getting started",
  "三步进入你的 Codex": "Start using Codex in three steps",
  "安装": "Install",
  "选择 Windows 安装程序或对应芯片的 macOS DMG。": "Choose the Windows installer or the macOS DMG for your chip.",
  "配置": "Configure",
  "打开管理工具，确认 Codex 路径，再设置供应商和需要的增强。": "Open the manager, confirm the Codex path, then configure your provider and desired enhancements.",
  "启动": "Launch",
  "以后从 Codex++ 入口启动，已保存的配置会自动加载。": "Launch from the Codex++ entry point from then on and your saved configuration will load automatically.",
  "下载 Codex++": "Download Codex++",
  "选择你的平台": "Choose your platform",
  "当前稳定版": "Current stable release",
  "。安装包由 GitHub Actions 从公开源码自动构建。": ". Installers are built automatically from public source by GitHub Actions.",
  "选择操作系统": "Choose an operating system",
  "Mac Apple 芯片": "Mac Apple silicon",
  "Windows 安装程序": "Windows installer",
  "包含 Codex++ 启动器和管理工具，并创建桌面与开始菜单入口。": "Includes the Codex++ launcher and manager, with desktop and Start menu shortcuts.",
  "正在读取文件信息": "Loading file details",
  "下载安装程序": "Download installer",
  "Mac Apple 芯片版": "Mac Apple silicon",
  "适用于 M1、M2、M3、M4 及后续 Apple 芯片 Mac，安装到 Applications 即可。": "For Macs with M1, M2, M3, M4, and later Apple chips. Install it in Applications.",
  "下载 Apple 芯片版": "Download Apple silicon build",
  "Mac Intel 版": "Mac Intel build",
  "适用于 Intel 处理器 Mac。若“关于本机”显示 Apple 芯片，请选择上一个版本。": "For Intel-based Macs. If About This Mac shows Apple silicon, choose the previous option.",
  "下载 Intel 版": "Download Intel build",
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
  "macOS 提示应用已损坏怎么办？": "What if macOS says the app is damaged?",
  "当前社区构建使用 ad-hoc 签名。请按项目 README 中的 macOS 安装说明处理 Gatekeeper 提示，并只从本项目 Release 下载。": "Community builds currently use ad-hoc signing. Follow the macOS installation instructions in the README for Gatekeeper prompts, and download only from this project's releases.",
  "把 Codex 调整成适合你的工具": "Shape Codex into the tool you need",
  "开源、可配置、可退出。先从管理工具开始。": "Open source, configurable, and reversible. Start with the manager.",
  "反馈问题": "Report an issue",
  "社区维护的 Codex 桌面增强与管理工具。": "A community-maintained desktop enhancement and management tool for Codex."
};

const pageMetadata = {
  zh: {
    title: "Codex++ - Codex 桌面增强与管理工具",
    description: "Codex++ 是面向 OpenAI Codex / ChatGPT 桌面应用的开源启动器与管理工具，提供供应商切换、会话管理、插件与脚本市场、界面增强等能力。",
    ogTitle: "Codex++ - 让 Codex 按你的方式工作",
    ogDescription: "管理供应商、模型、会话、插件、脚本和界面增强。适用于 Windows 与 macOS。",
  },
  en: {
    title: "Codex++ - Desktop enhancements and management for Codex",
    description: "Codex++ is an open-source launcher and manager for the OpenAI Codex / ChatGPT desktop app, with provider switching, session management, plugins, scripts, themes, and interface enhancements.",
    ogTitle: "Codex++ - Make Codex work your way",
    ogDescription: "Manage providers, models, sessions, plugins, scripts, themes, and interface enhancements on Windows and macOS.",
  },
};

const translatedTextNodes = new Map();
const translatedAttributes = new Map();
let currentLanguage = document.documentElement.dataset.language === "en" ? "en" : "zh";
let detectedPlatform = "windows";
let releaseAssets = new Map();
let repositoryStars = null;

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

  const metadata = pageMetadata[currentLanguage];
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

  releaseAssets.forEach(({ asset, version }, platform) => setPlatformAsset(platform, asset, version));

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
    fallbackName: `CodexPlusPlus-${fallbackVersion}-windows-x64-setup.exe`,
    type: "EXE",
  },
  "mac-arm": {
    assetPattern: /macos-arm64\.dmg$/i,
    fallbackName: `CodexPlusPlus-${fallbackVersion}-macos-arm64.dmg`,
    type: "DMG",
  },
  "mac-intel": {
    assetPattern: /macos-x64\.dmg$/i,
    fallbackName: `CodexPlusPlus-${fallbackVersion}-macos-x64.dmg`,
    type: "DMG",
  },
};

const formatBytes = (bytes) => {
  if (!Number.isFinite(bytes) || bytes <= 0) return "";
  const megabytes = bytes / (1024 * 1024);
  return `${megabytes.toFixed(megabytes >= 100 ? 0 : 1)} MB`;
};

const directAssetUrl = (version, fileName) =>
  `https://github.com/${repository}/releases/download/v${version}/${fileName}`;

const setPlatformAsset = (platform, asset, version) => {
  const config = platformConfig[platform];
  const link = document.querySelector(`[data-download="${platform}"]`);
  const meta = document.querySelector(`[data-asset-meta="${platform}"]`);
  if (!config || !link || !meta) return;

  if (asset) {
    link.href = asset.browser_download_url;
    const size = formatBytes(asset.size);
    meta.textContent = `${config.type}${size ? ` · ${size}` : ""} · GitHub Release`;
    return;
  }

  const fallbackName = config.fallbackName.replace(fallbackVersion, version);
  link.href = directAssetUrl(version, fallbackName);
  meta.textContent = `${config.type} · GitHub Release`;
};

const updateRelease = async () => {
  try {
    const response = await fetch(`https://api.github.com/repos/${repository}/releases/latest`, {
      headers: { Accept: "application/vnd.github+json" },
    });
    if (!response.ok) throw new Error(`GitHub release API returned ${response.status}`);

    const release = await response.json();
    const version = String(release.tag_name || `v${fallbackVersion}`).replace(/^v/i, "");
    document.querySelectorAll("[data-version]").forEach((element) => {
      element.textContent = `v${version}`;
    });

    Object.entries(platformConfig).forEach(([platform, config]) => {
      const asset = (release.assets || []).find((item) => config.assetPattern.test(item.name));
      releaseAssets.set(platform, { asset, version });
      setPlatformAsset(platform, asset, version);
    });
  } catch (error) {
    Object.keys(platformConfig).forEach((platform) => setPlatformAsset(platform, null, fallbackVersion));
    console.info("Using bundled release links", error);
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

const platformFromDevice = () => {
  const platform = navigator.userAgentData?.platform || navigator.platform || navigator.userAgent;
  if (/mac/i.test(platform)) return "mac-arm";
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
updateRelease();
updateRepositoryStats();

window.addEventListener("error", (event) => {
  if (event.target instanceof HTMLImageElement) {
    event.target.closest("figure")?.classList.add("image-unavailable");
  }
}, true);
