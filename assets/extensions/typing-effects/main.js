  // 打字特效拓展的入口：拓展启用时装上全局输入监听，停用 / 热重载时摘掉并清掉画布。
  // 特效样式来自本拓展的配置（config.effect），改了设置原地生效，不必整体重载。
  codexPlus.extension((ctx) => {
    const effects = createCodexPlusTypingEffects(() => ctx.config.effect);
    ctx.effect(() => {
      effects.install();
      return () => effects.uninstall();
    });
    ctx.onConfigUpdate(() => {
      effects.sync();
      return true;
    });
  });
