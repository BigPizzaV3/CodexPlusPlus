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
   *
   * 补数据的通用部分（codexPlusOfflineStatsigPatch）也供「启动时没联网，分支对话卡在
   * execution-config-loading」的兜底使用（#2898，见 32-fork-readiness-fallback.js）。
   */
  const codexPlusOfflineAutoReviewGate = "3902016271";

  function codexPlusOfflineAutoReviewEnabled() {
    return codexPlusBackendSettingsLoaded === true && codexPlusSettings().offlineAutoReview === true;
  }

  function codexPlusOfflineStatsigClients() {
    const root = window.__STATSIG__ || globalThis.__STATSIG__;
    if (!root || typeof root !== "object") return [];
    const clients = [root.firstInstance, typeof root.instance === "function" ? root.instance() : null];
    if (root.instances && typeof root.instances === "object") clients.push(...Object.values(root.instances));
    return clients.filter((client, index, array) => client && typeof client === "object" && array.indexOf(client) === index);
  }

  // 补进去的数据带这个顶层标记（Statsig 原样保留解析后的 JSON，getContext().values 读得到）。
  // 认得出是自己补的，才能在「分支对话兜底」（#2898）和「替我审批」先后补数据时合并闸门，而不是谁先补谁说了算。
  const codexPlusOfflineStatsigMarker = "codex_plus_offline";

  // 与 Statsig SDK 的 _DJB2Object 同算法（键排序后的 JSON 做 DJB2），用来复算数据适配器里的 fullUserHash。
  function codexPlusStatsigSortedObject(value) {
    if (value == null) return null;
    const sorted = {};
    for (const key of Object.keys(value).sort()) {
      const item = value[key];
      sorted[key] = item !== null && typeof item === "object" && !Array.isArray(item) ? codexPlusStatsigSortedObject(item) : item;
    }
    return sorted;
  }

  function codexPlusStatsigUserHash(user) {
    if (!user) return null;
    const text = JSON.stringify(codexPlusStatsigSortedObject(user));
    let hash = 0;
    for (let index = 0; index < text.length; index++) {
      hash = (hash << 5) - hash + text.charCodeAt(index);
      hash &= hash;
    }
    return String(hash >>> 0);
  }

  /**
   * 现有远程数据原生认不认（#3162）。Codex 26.1111 起渲染进程只在数据「确实属于当前用户」时
   * 才把执行配置发布给主进程：数据自带 user 时按 user 比对；否则要求数据适配器里这份数据的
   * fullUserHash 等于当前 user 的哈希。Statsig 拉不到、从本地缓存读出旧数据时，user 里每次启动
   * 都变的字段（如 codex_app_session_id）让哈希对不上，原生就当作「没有远程数据」处理。
   * 返回 true = 有数据但原生不认；数据自带 user、或判断不了时一律返回 false（不碰）。
   */
  function codexPlusOfflineStatsigValuesUnverified(client, context) {
    const values = context?.values;
    if (values == null || typeof values !== "object" || values.user != null) return false;
    const adapter = client?.dataAdapter;
    if (typeof adapter?.getDataSync !== "function") return false;
    let entry;
    try {
      entry = adapter.getDataSync(context.user);
    } catch {
      return false;
    }
    if (entry == null) return true;
    const expected = codexPlusStatsigUserHash(context.user);
    return typeof entry.fullUserHash === "string" && expected != null && entry.fullUserHash !== expected;
  }

  /**
   * 客户端 Ready 但完全没有远程数据（启动时没联网 / 拉不到 Statsig）时，补一份只含指定闸门的最小数据；
   * 已经是自己补的数据时只合并缺的闸门。拿到了真实数据就完全不碰。真的写了返回 true。
   * gates：{ 闸门名: 布尔值 }，可以为空——空数据也能让原生把 Statsig 视为「已就绪、全部按关处理」。
   * options.adoptUnverified：有数据但原生不认（缓存里的旧数据，见上）时，原样沿用这份数据，
   * 只补上当前 user 并打标记，原生据此认作当前用户的数据（#3162）。闸门值一个不改。
   */
  function codexPlusOfflineStatsigPatch(client, gates = {}, options = {}) {
    if (!client || client.loadingStatus !== "Ready") return false;
    let context;
    try {
      context = client.getContext();
    } catch {
      return false;
    }
    if (!context) return false;
    const current = context.values;
    const owned = current != null && current[codexPlusOfflineStatsigMarker] === true;
    if (current != null && !owned
      && !(options.adoptUnverified === true && codexPlusOfflineStatsigValuesUnverified(client, context))) return false;
    // 自己补的、或沿用的缓存数据保留原有字段；完全没数据时从空数据起步。
    const base = current != null && typeof current === "object" ? current : null;
    const featureGates = base?.feature_gates && typeof base.feature_gates === "object"
      ? { ...base.feature_gates }
      : {};
    let changed = !owned;
    // init-v2 格式的闸门结构不同，不往里写；沿用这类数据时只补 user。
    const gatesWritable = base?.response_format !== "init-v2";
    for (const [name, value] of gatesWritable ? Object.entries(gates) : []) {
      if (featureGates[name]?.value === value) continue;
      featureGates[name] = {
        name,
        value,
        rule_id: "codex-plus:offline-statsig",
        secondary_exposures: [],
        id_type: "userID",
      };
      changed = true;
    }
    if (!changed) return false;
    const adapter = client.dataAdapter;
    if (typeof adapter?.setData !== "function" || typeof client.updateUserSync !== "function") return false;
    const values = {
      dynamic_configs: {},
      layer_configs: {},
      param_stores: {},
      time: Date.now(),
      hash_used: "none",
      ...base,
      feature_gates: featureGates,
      // Statsig 只接受 has_updates 为 true 的数据。
      has_updates: true,
      user: context.user,
      [codexPlusOfflineStatsigMarker]: true,
    };
    adapter.setData(JSON.stringify(values));
    client.updateUserSync(context.user);
    return true;
  }

  /** 补一次；真的补了返回 true。客户端还在加载、或已经有真实数据时什么都不做。 */
  function codexPlusOfflineAutoReviewPatch(client) {
    return codexPlusOfflineStatsigPatch(client, { [codexPlusOfflineAutoReviewGate]: true });
  }

  function codexPlusOfflineAutoReviewTick() {
    if (!codexPlusOfflineAutoReviewEnabled()) return;
    for (const client of codexPlusOfflineStatsigClients()) {
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
