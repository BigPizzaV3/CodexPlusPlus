  // 打字特效拓展的入口：拓展启用时装上全局输入监听，停用 / 热重载时摘掉并清掉画布。
  // 特效样式来自本拓展的配置（config.effect），改了设置原地生效，不必整体重载。
  codexPlus.extension((ctx) => {
    // 内核先调 onConfigUpdate、再把 ctx.config 换成新值，所以样式记在这里，改设置时从 next 取。
    let effect = ctx.config.effect;
    const effects = createCodexPlusTypingEffects(() => effect);
    ctx.effect(() => {
      // 旧版注入里的内置打字特效（拆包前）还在页面里时先停掉它，否则两套粒子叠在一起，换样式看不出区别。
      window.__codexPlusTypingEffectsRuntime?.dispose?.();
      effects.install();
      return () => effects.uninstall();
    });
    ctx.onConfigUpdate((next) => {
      effect = next?.effect;
      effects.sync();
      return true;
    });
  });
