  // Agent 桌宠拓展的入口：图片桌宠（pet.js）与大肥鱼（whale.js）二选一，按本拓展的 config.kind 决定。
  // 只用公开接口：overlay 插槽给容器、onScan 在 Codex 重建界面后补回、onConfigUpdate 在管理器改设置后原地生效。
  codexPlus.extension((ctx) => {
    const pet = createCodexPlusAgentPet(ctx);
    const whale = createCodexPlusWhaleWidget(ctx);
    const sync = () => {
      pet.sync();
      whale.sync();
    };
    ctx.registerSlot({
      slot: "overlay",
      order: 10,
      render: ({ container }) => {
        pet.setHost(container);
        return () => pet.setHost(null);
      },
    });
    ctx.onScan(sync, { phase: "deferred" });
    ctx.listen(document, "visibilitychange", sync);
    // ctx.config 已原地换成新值，按新设置对齐即可，不必整体热重载。
    ctx.onConfigUpdate(() => {
      sync();
      return true;
    });
    ctx.effect(() => {
      sync();
      return () => {
        pet.dispose();
        whale.dispose();
      };
    });
    // 给测试与诊断看的只读状态。
    window.__codexPlusAgentPetExtension = Object.freeze({
      get pet() { return pet.state; },
      get whale() { return whale.state; },
    });
  });
