const assert = require("node:assert/strict");
const { readFileSync } = require("node:fs");
const vm = require("node:vm");

const renderer = readFileSync(`${__dirname}/renderer-inject/90-action-groups.js`, "utf8");
const start = renderer.indexOf("  // 旧版（26.9xx 之前）内容容器类名清单");
const end = renderer.indexOf("  function codexServiceTierBadgeVisibleElement(", start);
assert.ok(start >= 0 && end > start);
const source = renderer.slice(start, end);
const restoreStart = renderer.indexOf("  function conversationViewRememberOriginals(");
const restoreEnd = renderer.indexOf("  function conversationViewResetOwnOffset(", restoreStart);
const resolveStart = renderer.indexOf("  function conversationViewObserveIfNeeded(");
const resolveEnd = renderer.indexOf("  function conversationViewAlignNow(", resolveStart);
assert.ok(restoreStart >= 0 && restoreEnd > restoreStart && resolveStart >= 0 && resolveEnd > resolveStart);
const contentClasses = "mx-auto w-full max-w-(--thread-content-max-width) px-toolbar relative flex shrink-0 flex-col pb-8";
const widthClasses = "mx-auto w-full max-w-(--thread-body-max-width)";

class Element {
  constructor(tag = "div", classes = "", attrs = {}) {
    this.tagName = tag.toUpperCase(); this.className = classes; this.attrs = attrs;
    this.children = []; this.parentElement = null; this.style = {}; this.dataset = {};
    this.vars = {}; this.computedMaxWidth = "none"; this.isConnected = true;
  }
  matches(selector) {
    return selector.split(",").some(part => {
      const value = part.trim();
      const taggedAttribute = /^([a-z]+)(\[.*\])$/.exec(value);
      if (taggedAttribute) return this.tagName.toLowerCase() === taggedAttribute[1] && this.matches(taggedAttribute[2]);
      if (value.startsWith(".")) return this.className.split(/\s+/).includes(value.slice(1));
      if (value.startsWith("#")) return this.attrs.id === value.slice(1);
      if (value.startsWith("[")) {
        const match = /^\[([^=\]]+)(?:=["']?([^"'\]]*)["']?)?\]$/.exec(value);
        assert.ok(match, `unsupported selector: ${value}`);
        return Object.hasOwn(this.attrs, match[1]) && (match[2] === undefined || this.attrs[match[1]] === match[2]);
      }
      return this.tagName.toLowerCase() === value.toLowerCase();
    });
  }
  querySelectorAll(selector) {
    return this.children.flatMap(child => [child, ...child.querySelectorAll("*")])
      .filter(child => selector === "*" || child.matches(selector));
  }
  querySelector(selector) { return this.querySelectorAll(selector)[0] || null; }
  closest(selector) { return this.matches(selector) ? this : this.parentElement?.closest(selector) || null; }
  contains(other) { return this === other || this.children.some(child => child.contains(other)); }
  getAttribute(name) { return this.attrs[name] ?? null; }
  append(child) { child.parentElement = this; this.children.push(child); return child; }
}

function fixture() {
  const root = new Element("body");
  const document = {
    body: root, activeElement: null,
    querySelector: selector => root.querySelector(selector),
    querySelectorAll: selector => root.querySelectorAll(selector),
  };
  const context = vm.createContext({ document, visibleElement: node => node.isConnected && node.attrs.hidden === undefined,
    selectors: { conversationViewContentAnchor: "[data-thread-user-message-navigation-content]", conversationViewScrollContainer: ".thread-scroll-container", conversationViewFooter: "[data-thread-scroll-footer]" },
    getComputedStyle: node => {
      const entries = Object.keys(node.vars);
      return { ...Object.fromEntries(entries.map((key, index) => [index, key])), length: entries.length,
        maxWidth: node.computedMaxWidth, getPropertyValue: name => node.vars[name] || "" };
    },
  });
  vm.runInContext(`${source}\n${renderer.slice(restoreStart, restoreEnd)}\n${renderer.slice(resolveStart, resolveEnd)}
    this.api = { content: conversationViewFindContentEl, composer: conversationViewFindComposerEl,
      resolve: conversationViewResolveTargets, remember: conversationViewRememberOriginals, state: conversationViewState };`, context);
  return { root, api: context.api };
}

// 历史宽度变量在整个布局树上继承，不能把它当作节点自己的 max-width 声明。
{
  const { root, api } = fixture();
  const scroll = root.append(new Element("div", "thread-scroll-container"));
  const layout = scroll.append(new Element("div", "h-full flex"));
  layout.vars["--thread-body-max-width"] = "900px";
  const content = layout.append(new Element("div", "renamed-content"));
  content.vars["--thread-body-max-width"] = "900px"; content.computedMaxWidth = "900px";
  assert.equal(api.content(), content, "inherited variables must not select the layout wrapper");
}
// 没有会话 scope 时，右侧 review pane 的同形工具类不能成为正文或作曲器。
{
  const { root, api } = fixture();
  const review = root.append(new Element("div", "", { "data-summary-panel-variant": "review" }));
  review.append(new Element("div", widthClasses));
  assert.equal(api.content(), null); assert.equal(api.composer(), null);
}
// 显式旧类名也不能把包含侧栏/工具 pane 的布局祖先当作正文。
{
  const { root, api } = fixture();
  const scroll = root.append(new Element("div", "thread-scroll-container"));
  const layout = scroll.append(new Element("div", contentClasses));
  layout.append(new Element("aside", "", { id: "app-shell-sidebar" }));
  const content = layout.append(new Element("div", "", { "data-thread-user-message-navigation-content": "" }));
  assert.equal(api.content(), content, "layout ancestor must be rejected even with matching legacy classes");
}
// 嵌套侧栏/工具 pane 的宽度后代同样不能抢在正文锚点前被选中。
for (const attrs of [{ id: "app-shell-sidebar" }, { "data-summary-panel-variant": "review" }]) {
  const { root, api } = fixture();
  const scroll = root.append(new Element("div", "thread-scroll-container"));
  scroll.append(new Element("div", "", attrs)).append(new Element("div", contentClasses));
  const content = scroll.append(new Element("div", "", { "data-thread-user-message-navigation-content": "" }));
  assert.equal(api.content(), content);
}
// 整个对话布局同时包含正文与页脚，不能只因它也有 max-width 就收窄整个布局。
{
  const { root, api } = fixture();
  const scroll = root.append(new Element("div", "thread-scroll-container"));
  const layout = scroll.append(new Element("div", contentClasses));
  const content = layout.append(new Element("div", "", { "data-thread-user-message-navigation-content": "" }));
  layout.append(new Element("div", widthClasses, { "data-thread-scroll-footer": "true" })).append(new Element("div", widthClasses));
  assert.equal(api.content(), content);
}
// 正常旧版、锚点新版与页脚内作曲器保留；不得选到页脚包裹层本身。
for (const modern of [false, true]) {
  const { root, api } = fixture();
  const scroll = root.append(new Element("div", "thread-scroll-container"));
  const content = scroll.append(new Element("div", modern ? "new-content" : contentClasses,
    modern ? { "data-thread-user-message-navigation-content": "" } : {}));
  const footer = scroll.append(new Element("div", widthClasses, { "data-thread-scroll-footer": "true" }));
  const composer = footer.append(new Element("div", widthClasses));
  assert.equal(api.content(), content); assert.equal(api.composer(), composer);
}
// 新版 sticky composer 可与正文 scroller 互为兄弟，仍属于同一个局部会话 pane。
{
  const { root, api } = fixture();
  const pane = root.append(new Element("main"));
  pane.append(new Element("header")).append(new Element("button", "codex-session-share-button", { "data-codex-plus-ext": "session-share" }));
  const scroll = pane.append(new Element("div", "thread-scroll-container"));
  const content = scroll.append(new Element("div", "", { "data-thread-user-message-navigation-content": "" }));
  const footer = pane.append(new Element("div", widthClasses, { "data-thread-scroll-footer": "true" }));
  const composer = footer.append(new Element("div", widthClasses));
  composer.append(new Element("button", "", { "data-codex-plus-ext": "builtin-dictation" }));
  assert.equal(api.content(), content); assert.equal(api.composer(), composer);
  api.resolve(); assert.equal(api.state.composerEl, composer, "layout consumers receive the same scoped composer");
}
// 另一个 review pane 的 footer 不能仅凭全局唯一而被关联到正文滚动区。
{
  const { root, api } = fixture();
  const pane = root.append(new Element("main"));
  pane.append(new Element("div", "thread-scroll-container"));
  const review = root.append(new Element("div", "", { "data-summary-panel-variant": "review" }));
  review.append(new Element("div", widthClasses, { "data-thread-scroll-footer": "true" })).append(new Element("div", widthClasses));
  assert.equal(api.composer(), null);
}
// 无页脚标记时，明确的原生输入框锚点仍可支持新版和首页 composer。
for (const hasScroller of [false, true]) {
  const { root, api } = fixture();
  const pane = root.append(new Element("main"));
  const host = hasScroller ? pane.append(new Element("div", "thread-scroll-container")) : pane;
  const composer = host.append(new Element("div", widthClasses));
  const native = composer.append(new Element("div", "", { "data-codex-composer-root": "true" }));
  native.append(new Element("textarea"));
  composer.append(new Element("aside", "rounded-3xl"));
  assert.equal(api.composer(), composer, "a composer status aside is not a sidebar");
}
// 旧版首页无scroller/原生root标记，但明确旧宽度类名加真实编辑器仍支持。
{
  const { root, api } = fixture();
  const composer = root.append(new Element("div", "relative z-10 flex flex-col mx-auto w-full max-w-(--thread-content-max-width) px-toolbar"));
  composer.append(new Element("textarea"));
  assert.equal(api.composer(), composer);
}
// 无footer/原生root标记的新类名必须以明确编辑器确认，不能选到消息内容盒。
{
  const { root, api } = fixture();
  const scroll = root.append(new Element("div", "thread-scroll-container"));
  scroll.append(new Element("div", widthClasses, { "data-thread-user-message-navigation-content": "" }));
  const composer = scroll.append(new Element("div", widthClasses));
  composer.append(new Element("div", "ProseMirror", { contenteditable: "true" }));
  assert.equal(api.composer(), composer);
}
// 歧义页脚不能落回完整旧类名，把第一个页脚包裹层当作 composer。
{
  const { root, api } = fixture();
  const scroll = root.append(new Element("div", "thread-scroll-container"));
  for (let index = 0; index < 2; index++) {
    scroll.append(new Element("div", "relative z-10 flex flex-col mx-auto w-full max-w-(--thread-content-max-width) px-toolbar",
      { "data-thread-scroll-footer": "true" })).append(new Element("div", widthClasses));
  }
  assert.equal(api.composer(), null);
}
// 多个可见会话时宁可不改宽度，也不能把别的会话/工具栏当作当前目标。
{
  const { root, api } = fixture();
  for (let index = 0; index < 2; index++) {
    const scroll = root.append(new Element("div", "thread-scroll-container"));
    scroll.append(new Element("div", "", { "data-thread-user-message-navigation-content": "" }));
    scroll.append(new Element("div", "", { "data-thread-scroll-footer": "true" })).append(new Element("div", widthClasses));
  }
  assert.equal(api.content(), null); assert.equal(api.composer(), null);
}
// DOM 仍连接但 scope 已变化时，释放旧宽度并恢复原样式，不缓存失效目标。
{
  const { root, api } = fixture();
  const scroll = root.append(new Element("div", "thread-scroll-container"));
  const content = scroll.append(new Element("div", "", { "data-thread-user-message-navigation-content": "" }));
  content.style.width = "75%"; content.style.maxWidth = "600px";
  api.resolve(); assert.equal(api.state.contentEl, content);
  api.remember(content); content.style.width = "100%"; content.style.maxWidth = "900px";
  scroll.className = "review-scroll";
  api.resolve();
  assert.equal(api.state.contentEl, null);
  assert.equal(content.style.width, "75%"); assert.equal(content.style.maxWidth, "600px");
  assert.equal(api.state.elements.size, 0);
}
console.log("conversation view scope and inherited width regressions passed");
