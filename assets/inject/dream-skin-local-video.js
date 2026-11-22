((runtime, source) => {
  if (!/^data:video\/(mp4|webm);base64,/.test(source)) return;
  const video = document.createElement("video");
  video.setAttribute("data-codex-plus-ext", "dream-skin-video");
  video.setAttribute("aria-hidden", "true");
  video.muted = video.defaultMuted = video.loop = video.playsInline = true;
  video.disablePictureInPicture = true;
  video.tabIndex = -1;
  video.preload = "metadata";
  const motion = matchMedia("(prefers-reduced-motion: reduce)");
  const selector = 'main:is([data-app-shell-main-surface],.main-surface,[class*="_MainContentSurface_"]):not([data-app-shell-active-page="false"] *)';
  let host;
  let disposed = false;
  const detach = () => {
    video.parentElement?.removeAttribute("data-codex-plus-skin-video-host");
    video.remove();
  };
  const sync = () => {
    if (disposed) return;
    if (!host?.isConnected || !host.matches(selector)) host = document.querySelector(selector);
    const off = document.documentElement.dataset.dreamTaskMode === "off" && !host?.querySelector('[data-composer-placement="home"]');
    if (!host || off) { video.pause(); detach(); return; }
    if (host !== video.parentElement) {
      detach();
      host.setAttribute("data-codex-plus-skin-video-host", "");
      host.appendChild(video);
    }
    if (document.hidden || motion.matches) video.pause();
    else if (video.paused) video.play().catch(() => {});
  };
  const observer = new MutationObserver((records) => {
    if (!host?.isConnected || !video.isConnected || records.some((record) => record.type === "attributes")) sync();
  });
  const cleanup = () => {
    if (disposed) return;
    disposed = true;
    observer.disconnect();
    document.removeEventListener("visibilitychange", sync);
    motion.removeEventListener("change", sync);
    video.removeEventListener("error", cleanup);
    video.pause();
    video.removeAttribute("src");
    video.load();
    detach();
  };
  const previousCleanup = runtime.cleanup;
  runtime.cleanup = (...args) => { cleanup(); return previousCleanup?.apply(runtime, args); };
  observer.observe(document.body, { subtree: true, childList: true, attributes: true, attributeFilter: ["data-app-shell-active-page"] });
  observer.observe(document.documentElement, { attributes: true, attributeFilter: ["data-dream-task-mode"] });
  document.addEventListener("visibilitychange", sync);
  motion.addEventListener("change", sync);
  video.addEventListener("error", cleanup);
  video.src = source;
  sync();
})(state, window.__CODEX_PLUS_DREAM_SKIN_ART__);
