((cssText, artDataUrl) => {
  const STATE_KEY = "__CODEX_GLASS_VISION_SKIN_STATE__";
  const STYLE_ID = "codex-glass-vision-skin-style";
  const CHROME_ID = "codex-glass-vision-skin-chrome";
  const VERSION = "2.1.0";
  window.__CODEX_GLASS_VISION_SKIN_DISABLED__ = false;

  const previous = window[STATE_KEY];
  previous?.observer?.disconnect();
  if (previous?.timer) clearInterval(previous.timer);
  if (previous?.scheduler?.timeout) clearTimeout(previous.scheduler.timeout);

  const isAvatarOverlay = document.documentElement?.classList.contains("compact-window") &&
    new URLSearchParams(window.location.search).get("initialRoute") === "/avatar-overlay";
  if (isAvatarOverlay) {
    if (previous?.cleanup) previous.cleanup();
    document.documentElement?.classList.remove("codex-glass-vision-skin");
    document.documentElement?.style.removeProperty("--glass-vision-art");
    document.getElementById(STYLE_ID)?.remove();
    document.getElementById(CHROME_ID)?.remove();
    delete window[STATE_KEY];
    return { installed: false, skipped: "avatar-overlay", version: VERSION };
  }

  const artUrl = previous?.artUrl || (() => {
    const comma = artDataUrl.indexOf(",");
    const binary = atob(artDataUrl.slice(comma + 1));
    const bytes = new Uint8Array(binary.length);
    for (let index = 0; index < binary.length; index += 1) bytes[index] = binary.charCodeAt(index);
    return URL.createObjectURL(new Blob([bytes], { type: "image/png" }));
  })();

  const writeStyle = (style) => {
    style.textContent = cssText;
    style.dataset.glassVisionVersion = VERSION;
  };

  const ensure = () => {
    if (window.__CODEX_GLASS_VISION_SKIN_DISABLED__) return;
    const root = document.documentElement;
    if (!root) return;

    root.classList.add("codex-glass-vision-skin");
    root.style.setProperty("--glass-vision-art", `url("${artUrl}")`);

    let style = document.getElementById(STYLE_ID);
    if (!style) {
      style = document.createElement("style");
      style.id = STYLE_ID;
      (document.head || root).appendChild(style);
    }
    if (style.dataset.glassVisionVersion !== VERSION || style.textContent !== cssText) writeStyle(style);

    const shellMain = document.querySelector("main.main-surface") || document.querySelector("main");
    const home = document.querySelector('[role="main"]:has([data-testid="home-icon"])');
    for (const candidate of document.querySelectorAll('[role="main"].glass-vision-home')) {
      if (candidate !== home) candidate.classList.remove("glass-vision-home");
    }
    home?.classList.add("glass-vision-home");

    if (!shellMain || !document.body) return;
    shellMain.classList.toggle("glass-vision-home-shell", Boolean(home));
    shellMain.classList.toggle("glass-vision-task-shell", !home);

    let chrome = document.getElementById(CHROME_ID);
    if (!chrome || chrome.parentElement !== document.body) {
      chrome?.remove();
      chrome = document.createElement("div");
      chrome.id = CHROME_ID;
      chrome.setAttribute("aria-hidden", "true");
      chrome.innerHTML = `
        <div class="glass-vision-brand">
          <span class="glass-vision-orbit-mark"><i></i></span>
          <span><b>GLASS VISION</b><small>SILVER BLUE · CELESTIAL</small></span>
        </div>
        <div class="glass-vision-status"><i></i><span>CRYSTAL FIELD</span></div>
        <div class="glass-vision-atmosphere"><i></i><i></i><i></i><i></i><i></i><i></i><i></i><i></i></div>
        <div class="glass-vision-orbit-lines"><i></i><i></i><i></i></div>
        <div class="glass-vision-prism"></div>`;
      document.body.appendChild(chrome);
    }

    const shellBox = shellMain.getBoundingClientRect();
    chrome.style.left = `${Math.round(shellBox.left)}px`;
    chrome.style.top = `${Math.round(shellBox.top)}px`;
    chrome.style.width = `${Math.round(shellBox.width)}px`;
    chrome.style.height = `${Math.round(shellBox.height)}px`;
    chrome.classList.toggle("glass-vision-home-shell", Boolean(home));
    chrome.classList.toggle("glass-vision-task-shell", !home);
  };

  const cleanup = () => {
    window.__CODEX_GLASS_VISION_SKIN_DISABLED__ = true;
    document.documentElement?.classList.remove("codex-glass-vision-skin");
    document.documentElement?.style.removeProperty("--glass-vision-art");
    document.querySelectorAll(".glass-vision-home").forEach((node) => node.classList.remove("glass-vision-home"));
    document.querySelectorAll(".glass-vision-home-shell").forEach((node) => node.classList.remove("glass-vision-home-shell"));
    document.querySelectorAll(".glass-vision-task-shell").forEach((node) => node.classList.remove("glass-vision-task-shell"));
    document.getElementById(STYLE_ID)?.remove();
    document.getElementById(CHROME_ID)?.remove();
    const state = window[STATE_KEY];
    state?.observer?.disconnect();
    if (state?.timer) clearInterval(state.timer);
    if (state?.scheduler?.timeout) clearTimeout(state.scheduler.timeout);
    if (state?.artUrl) URL.revokeObjectURL(state.artUrl);
    delete window[STATE_KEY];
    return true;
  };

  const scheduler = { timeout: null };
  const scheduleEnsure = () => {
    if (scheduler.timeout) clearTimeout(scheduler.timeout);
    scheduler.timeout = setTimeout(() => {
      scheduler.timeout = null;
      ensure();
    }, 180);
  };
  const observer = new MutationObserver(scheduleEnsure);
  observer.observe(document.documentElement, { childList: true, subtree: true });
  const timer = setInterval(ensure, 5000);
  window[STATE_KEY] = { ensure, cleanup, observer, timer, scheduler, artUrl, version: VERSION };
  ensure();
  return { installed: true, version: VERSION };
})(__GLASS_VISION_CSS_JSON__, __GLASS_VISION_ART_JSON__)
