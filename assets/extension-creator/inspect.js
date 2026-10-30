// 拓展创作 MCP 的只读页面查看：在 Codex 页面里执行，返回元素摘要与建议选择器。
// 由 Rust 侧以 `(${本文件})(${JSON 参数})` 求值，agent 只能传参数，不能传代码；脚本不改动页面。
(options) => {
  const limit = Math.min(Math.max(Number(options.limit) || 20, 1), 50);
  const includeCodexPlus = options.includeCodexPlus === true;
  // CSS Modules 生成的类名带构建哈希（如 _ComposerLayoutFooter_1hyg8_2），Codex 每次发版都可能变。
  const hashedClass = /^_[A-Za-z][\w]*_[a-z0-9]{4,6}_\d+$/;
  const ownSelector = "[data-codex-plus-ext], [data-codex-plus-page], .codex-plus-page-overlay, .codex-plus-page, #codex-plus-rail-nav, #codex-plus-rail-extensions, #codex-plus-rail-sponsor, #codex-plus-rail-tooltip, [data-codex-plus-ext-rail]";
  const isOurs = (el) => !!el.closest?.(ownSelector);
  const zoom = Number.parseFloat(getComputedStyle(document.documentElement).getPropertyValue("--codex-window-zoom")) || 1;

  const visible = (el) => {
    const rect = el.getBoundingClientRect();
    if (rect.width <= 0 || rect.height <= 0) return false;
    const style = getComputedStyle(el);
    return style.visibility !== "hidden" && style.display !== "none" && Number(style.opacity) !== 0;
  };
  const quote = (value) => JSON.stringify(String(value));
  const stableAttributes = (el) => {
    const out = {};
    for (const attr of el.attributes) {
      const name = attr.name;
      if (name === "class" || name === "style" || name === "d" || name.startsWith("on")) continue;
      // Radix 等运行时生成的 id 每次渲染都不同，没有参考价值。
      if (name === "id" && /^(radix-|:r|_r_)/.test(attr.value)) continue;
      if (["id", "role", "aria-label", "aria-labelledby", "title", "name", "type", "placeholder", "href", "contenteditable"].includes(name)
        || name.startsWith("data-") || name.startsWith("aria-")) {
        out[name] = attr.value.length > 120 ? `${attr.value.slice(0, 120)}…` : attr.value;
      }
    }
    return out;
  };
  const classInfo = (el) => {
    const all = String(el.getAttribute("class") || "").split(/\s+/).filter(Boolean);
    return { classes: all.filter((name) => !hashedClass.test(name)).slice(0, 12), hashedClasses: all.filter((name) => hashedClass.test(name)) };
  };
  const ownSelectorParts = (el) => {
    const tag = el.tagName.toLowerCase();
    const parts = [];
    const pick = (name) => el.getAttribute(name);
    if (pick("data-testid")) parts.push(`${tag}[data-testid=${quote(pick("data-testid"))}]`);
    if (pick("aria-label")) parts.push(`${tag}[aria-label=${quote(pick("aria-label"))}]`);
    if (el.id && !/^(radix-|:r|_r_)/.test(el.id)) parts.push(`#${CSS.escape(el.id)}`);
    for (const attr of el.attributes) {
      if (attr.name.startsWith("data-") && !["data-state", "data-testid"].includes(attr.name) && attr.value.length <= 60 && !/\d{4,}/.test(attr.value)) {
        parts.push(`${tag}[${attr.name}${attr.value ? `=${quote(attr.value)}` : ""}]`);
      }
    }
    if (pick("role")) parts.push(`${tag}[role=${quote(pick("role"))}]`);
    if (pick("placeholder")) parts.push(`${tag}[placeholder=${quote(pick("placeholder"))}]`);
    return parts;
  };
  /** 只用稳定属性拼选择器：先试元素自身，不唯一再加一层有稳定属性的祖先。 */
  const suggestSelector = (el) => {
    const count = (selector) => {
      try {
        return document.querySelectorAll(selector).length;
      } catch {
        return 0;
      }
    };
    const own = ownSelectorParts(el);
    for (const selector of own) if (count(selector) === 1) return { selector, matches: 1 };
    for (let parent = el.parentElement, depth = 0; parent && depth < 6; parent = parent.parentElement, depth += 1) {
      for (const outer of ownSelectorParts(parent)) {
        for (const inner of own.length ? own : [el.tagName.toLowerCase()]) {
          const selector = `${outer} ${inner}`;
          if (count(selector) === 1) return { selector, matches: 1 };
        }
      }
    }
    const fallback = own[0] || null;
    return { selector: fallback, matches: fallback ? count(fallback) : 0 };
  };
  const ancestry = (el) => {
    const out = [];
    for (let node = el.parentElement, depth = 0; node && node !== document.body && depth < 4; node = node.parentElement, depth += 1) {
      const attrs = stableAttributes(node);
      const label = Object.entries(attrs).slice(0, 3).map(([key, value]) => `${key}=${value}`).join(" ");
      out.push(`${node.tagName.toLowerCase()}${label ? `[${label}]` : ""}${classInfo(node).hashedClasses.length ? ` .${classInfo(node).hashedClasses[0]}` : ""}`);
    }
    return out;
  };
  const html = (el) => el.outerHTML
    .replace(/\sd="[^"]{40,}"/g, ' d="…"')
    .replace(/(<svg\b[^>]*>)[\s\S]*?(<\/svg>)/g, "$1…$2")
    .slice(0, 400);
  const describe = (el) => {
    const rect = el.getBoundingClientRect();
    return {
      tag: el.tagName.toLowerCase(),
      attributes: stableAttributes(el),
      ...classInfo(el),
      text: (el.innerText || el.textContent || "").trim().replace(/\s+/g, " ").slice(0, 80),
      visible: visible(el),
      rect: { x: Math.round(rect.x), y: Math.round(rect.y), width: Math.round(rect.width), height: Math.round(rect.height) },
      ...suggestSelector(el),
      ancestors: ancestry(el),
      html: html(el),
    };
  };
  const keep = (el) => (includeCodexPlus || !isOurs(el)) && (options.visibleOnly === false || visible(el));
  const page = {
    path: location.pathname + location.hash,
    title: document.title,
    viewport: { width: innerWidth, height: innerHeight },
    zoom,
    theme: document.documentElement.className.match(/\b(dark|light)\b/)?.[1] || getComputedStyle(document.documentElement).colorScheme,
  };

  if (typeof options.selector === "string" && options.selector.trim()) {
    let nodes;
    try {
      nodes = [...document.querySelectorAll(options.selector)];
    } catch (error) {
      return { page, error: `选择器无效：${error.message}` };
    }
    const kept = nodes.filter(keep);
    return { page, mode: "selector", total: nodes.length, matches: kept.slice(0, limit).map(describe) };
  }

  if (typeof options.query === "string" && options.query.trim()) {
    const needle = options.query.trim().toLowerCase();
    const interactive = "button, a, input, textarea, select, [role], [contenteditable=true], [aria-label], [title], [placeholder]";
    const hits = [];
    for (const el of document.querySelectorAll(interactive)) {
      if (!keep(el)) continue;
      const haystack = [el.getAttribute("aria-label"), el.getAttribute("title"), el.getAttribute("placeholder"), el.getAttribute("data-testid"),
        el.children.length <= 3 ? el.innerText : ""].filter(Boolean).join(" ").toLowerCase();
      if (haystack.includes(needle)) hits.push(el);
      if (hits.length >= limit) break;
    }
    return { page, mode: "query", matches: hits.map(describe) };
  }

  // 不给条件时返回关键区域概览，帮 agent 先认路。
  const first = (selector) => [...document.querySelectorAll(selector)].find((el) => keep(el)) || null;
  const composer = first(".ProseMirror, [contenteditable=true], textarea");
  let composerArea = composer;
  for (let i = 0; composerArea && i < 8; i += 1) {
    composerArea = composerArea.parentElement;
    if (composerArea && composerArea.querySelectorAll("button").length >= 2) break;
  }
  const buttonsIn = (root) => (root ? [...root.querySelectorAll("button")].filter(keep).slice(0, 12).map(describe) : []);
  const rail = first("nav[data-app-navigation-rail]");
  const header = first("header");
  return {
    page,
    mode: "landmarks",
    notes: [
      "发送按钮等元素可能只在特定状态下出现（例如输入框有文字时），查不到时换个状态再查，或用 MutationObserver 等它出现。",
      "hashedClasses 里的类名带构建哈希，Codex 发版后会变，不要依赖；优先用 selector 字段给出的稳定选择器。",
    ],
    composer: composer && describe(composer),
    composerButtons: buttonsIn(composerArea),
    rail: rail && { ...describe(rail), buttons: buttonsIn(rail) },
    header: header && { ...describe(header), buttons: buttonsIn(header) },
  };
}
