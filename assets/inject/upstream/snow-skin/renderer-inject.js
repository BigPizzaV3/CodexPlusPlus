((cssText, artDataUrl, skinVersion) => {
  const STATE_KEY = "__CODEX_DREAM_SKIN_STATE__";
  const STYLE_ID = "codex-dream-skin-style";
  const CHROME_ID = "codex-dream-skin-chrome";
  window.__CODEX_DREAM_SKIN_DISABLED__ = false;

  const previous = window[STATE_KEY];
  if (previous?.observer) previous.observer.disconnect();
  if (previous?.timer) clearInterval(previous.timer);
  if (previous?.scheduler?.timeout) clearTimeout(previous.scheduler.timeout);
  const previousArtUrl = previous?.artUrl;
  const artUrl = (() => {
    const comma = artDataUrl.indexOf(",");
    const binary = atob(artDataUrl.slice(comma + 1));
    const bytes = new Uint8Array(binary.length);
    for (let index = 0; index < binary.length; index += 1) bytes[index] = binary.charCodeAt(index);
    return URL.createObjectURL(new Blob([bytes], { type: "image/png" }));
  })();
  const existingStyle = document.getElementById(STYLE_ID);
  if (existingStyle) {
    existingStyle.textContent = cssText;
    existingStyle.dataset.dreamVersion = skinVersion;
  }

  const ensure = () => {
    if (window.__CODEX_DREAM_SKIN_DISABLED__) return;
    const root = document.documentElement;
    if (!root) return;
    root.classList.add("codex-dream-skin");
    root.style.setProperty("--dream-art", `url("${artUrl}")`);

    let style = document.getElementById(STYLE_ID);
    if (!style) {
      style = document.createElement("style");
      style.id = STYLE_ID;
      (document.head || root).appendChild(style);
    }
    if (style.dataset.dreamVersion !== skinVersion) {
      style.textContent = cssText;
      style.dataset.dreamVersion = skinVersion;
    }

    const shellMain = document.querySelector("main.main-surface") || document.querySelector("main");
    const homeCandidate = document.querySelector('[role="main"]:has([data-testid="home-icon"])');
    const homeHasClassicChrome = !!(
      homeCandidate
      && homeCandidate.querySelector('[data-feature="game-source"]')
      && (
        homeCandidate.querySelector('.group\\/home-suggestions')
        || homeCandidate.querySelector('[class*="home-suggestions"]')
        || homeCandidate.querySelector('[class*="_homeUtilityBar_"]')
      )
    );
    const home = homeHasClassicChrome ? homeCandidate : null;
    for (const candidate of document.querySelectorAll('[role="main"]')) {
      const isStructuredHome = candidate === home;
      const isSoftHome = candidate === homeCandidate && !home;
      candidate.classList.toggle("dream-home", isStructuredHome || isSoftHome);
      candidate.classList.toggle("dream-task", !(isStructuredHome || isSoftHome));
      if (isStructuredHome) {
        const hero = candidate.querySelector(":scope > div > div > div");
        const structured = !!(
          hero
          && candidate.querySelector('[data-feature="game-source"]')
          && hero.querySelector('[data-feature="game-source"], [data-testid="home-icon"]')
        );
        candidate.setAttribute("data-dream-home-layout", structured ? "structured" : "soft");
      } else {
        candidate.setAttribute("data-dream-home-layout", "soft");
      }
    }
    const utilityBars = new Set(home ? home.querySelectorAll('[class*="_homeUtilityBar_"]') : []);
    for (const candidate of document.querySelectorAll(`.${HOME_UTILITY_CLASS}`)) {
      if (!utilityBars.has(candidate)) candidate.classList.remove(HOME_UTILITY_CLASS);
    }
    for (const candidate of utilityBars) candidate.classList.add(HOME_UTILITY_CLASS);
    shellMain.classList.toggle("dream-home-shell", Boolean(homeCandidate));
    let chrome = document.getElementById(CHROME_ID);
    if (!chrome || chrome.parentElement !== document.body) {
      chrome?.remove();
      chrome = document.createElement("div");
      chrome.id = CHROME_ID;
      chrome.setAttribute("aria-hidden", "true");
      chrome.innerHTML = `
        <div class="dream-brand"><span class="dream-note">SKI</span><span><b>Snowline Codex</b><small>ice-blue training mode</small></span></div>
        <div class="dream-signature">Freeski focus</div>
        <div class="dream-sparkles"><i></i><i></i><i></i><i></i><i></i><i></i></div>
        <div class="dream-ribbon"><span>slopestyle</span><strong>double cork energy</strong><span>halfpipe</span></div>
        <div class="dream-polaroid"></div>`;
      document.body.appendChild(chrome);
    }
    const shellBox = shellMain.getBoundingClientRect();
    chrome.style.left = `${Math.round(shellBox.left)}px`;
    chrome.style.top = `${Math.round(shellBox.top)}px`;
    chrome.style.width = `${Math.round(shellBox.width)}px`;
    chrome.style.height = `${Math.round(shellBox.height)}px`;
    chrome.classList.toggle("dream-home-shell", Boolean(home));
  };

  const cleanup = () => {
    window.__CODEX_DREAM_SKIN_DISABLED__ = true;
    document.documentElement?.classList.remove("codex-dream-skin");
    document.documentElement?.style.removeProperty("--dream-art");
    document.querySelectorAll(".dream-home").forEach((node) => { node.classList.remove("dream-home"); node.removeAttribute("data-dream-home-layout"); });
    document.querySelectorAll(".dream-home-shell").forEach((node) => node.classList.remove("dream-home-shell"));
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
  window[STATE_KEY] = { ensure, cleanup, observer, timer, scheduler, artUrl, version: skinVersion };
  ensure();
  if (previousArtUrl && previousArtUrl !== artUrl) URL.revokeObjectURL(previousArtUrl);
  return { installed: true, version: skinVersion };
})(__DREAM_CSS_JSON__, __DREAM_ART_JSON__, __DREAM_VERSION_JSON__)
