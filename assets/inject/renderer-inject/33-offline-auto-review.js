  /**
   * 内置拓展「拿不到远程开关时补开『替我审批』」（#2817）。
   *
   * Codex 权限菜单里的「替我审批」（agentMode = guardian-approvals）是否出现，看的是 Statsig
   * 远程开关 `guardian_approval`（闸门 3902016271）。这份数据由 OpenAI 后端预置下发，国内网络下
   * 拉不到时 Statsig 客户端是 Ready 但没有任何数据（NoValues），所有远程开关都按关处理，入口随之消失。
   * 审批本身是一次普通的模型请求（固定模型名 codex-auto-review），走当前配置的供应商 / 网关，
   * 所以补上入口就能用。
   *
   * 做法：只在客户端完全没有数据时，往它的数据适配器里补一份只含这一个闸门的最小数据，其余开关
   * 仍然是「没下发 = 关」，和补之前一样。`setData` 只写内存、不写 localStorage，关掉开关后重启
   * Codex 即恢复原样；拿到了真实数据（有网络）就完全不碰。
   */
  const codexPlusOfflineAutoReviewGate = "3902016271";
  const codexPlusOfflineAutoReviewRule = "codex-plus:offline-auto-review";

  function codexPlusOfflineAutoReviewEnabled() {
    return codexPlusBackendSettingsLoaded === true && codexPlusSettings().offlineAutoReview === true;
  }

  function codexPlusOfflineAutoReviewClients() {
    const root = window.__STATSIG__ || globalThis.__STATSIG__;
    if (!root || typeof root !== "object") return [];
    const clients = [root.firstInstance, typeof root.instance === "function" ? root.instance() : null];
    if (root.instances && typeof root.instances === "object") clients.push(...Object.values(root.instances));
    return clients.filter((client, index, array) => client && typeof client === "object" && array.indexOf(client) === index);
  }

  /** 补一次；真的补了返回 true。客户端还在加载、或已经有数据时什么都不做。 */
  function codexPlusOfflineAutoReviewPatch(client) {
    if (!client || client.loadingStatus !== "Ready") return false;
    let context;
    try {
      context = client.getContext();
    } catch {
      return false;
    }
    if (!context || context.values != null) return false;
    const adapter = client.dataAdapter;
    if (typeof adapter?.setData !== "function" || typeof client.updateUserSync !== "function") return false;
    const values = {
      feature_gates: {
        [codexPlusOfflineAutoReviewGate]: {
          name: codexPlusOfflineAutoReviewGate,
          value: true,
          rule_id: codexPlusOfflineAutoReviewRule,
          secondary_exposures: [],
          id_type: "userID",
        },
      },
      dynamic_configs: {},
      layer_configs: {},
      param_stores: {},
      time: Date.now(),
      has_updates: true,
      hash_used: "none",
      user: context.user,
    };
    adapter.setData(JSON.stringify(values));
    client.updateUserSync(context.user);
    return true;
  }

  function codexPlusOfflineAutoReviewTick() {
    if (!codexPlusOfflineAutoReviewEnabled()) return;
    for (const client of codexPlusOfflineAutoReviewClients()) {
      try {
        if (codexPlusOfflineAutoReviewPatch(client)) {
          console.log("[OfflineAutoReview]", "Statsig 没有远程数据，已补开「替我审批」入口");
        }
      } catch (error) {
        console.warn("[OfflineAutoReview]", "补开失败：", error && error.message);
      }
    }
  }

  function codexPlusOfflineAutoReviewExtension(ctx) {
    codexPlusOfflineAutoReviewTick();
    // Statsig 可能晚于注入才 Ready，后台刷新失败也可能再次清空数据：低频复查，补过且仍有数据时是空操作。
    ctx.setInterval(codexPlusOfflineAutoReviewTick, 3000);
  }
