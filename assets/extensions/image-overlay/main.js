  // 背景图拓展的入口：图片从 Codex++ 读（页面拿不到本机文件），透明度与适配方式来自本拓展的配置。
  // 拓展启用时装上，改了设置原地重铺，停用 / 热重载时撤掉覆盖层并释放图片。
  codexPlus.extension((ctx) => {
    let generation = 0;
    const report = (event, data) => {
      void ctx.call("/diagnostics/log", { event, data }).catch(() => {});
    };
    // 内核先调 onConfigUpdate、再把 ctx.config 换成新值，所以改设置时把 next 传进来。
    const apply = async (config = ctx.config) => {
      const current = ++generation;
      let dataUrl = "";
      try {
        const result = await ctx.call("/image-overlay/image", {});
        dataUrl = typeof result?.dataUrl === "string" ? result.dataUrl : "";
      } catch (error) {
        report("image_overlay_load_failed", { message: String(error?.message || error) });
      }
      // 读图期间又改了设置或拓展已卸下：交给更新的那一次。
      if (current !== generation || ctx.disposed) return;
      const opacity = Number(config.opacity);
      installCodexPlusImageOverlay({
        enabled: !!dataUrl,
        dataUrl,
        opacity: Number.isFinite(opacity) ? opacity / 100 : 0.35,
        fitMode: config.fitMode,
      }, report);
    };
    const whenReady = () => {
      if (document.readyState !== "loading") {
        void apply();
        return;
      }
      ctx.listen(document, "DOMContentLoaded", () => void apply(), { once: true });
    };
    ctx.effect(() => {
      whenReady();
      return () => {
        generation += 1;
        installCodexPlusImageOverlay({ enabled: false });
      };
    });
    ctx.onConfigUpdate((next) => {
      void apply(next);
      return true;
    });
  });
