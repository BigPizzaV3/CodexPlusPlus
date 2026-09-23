(() => {
  // 只修正本机自定义供应商的 composer 门禁，不伪造账户额度或修改发送按钮。
  function permitsExternalApi(settings, hostId) {
    if (hostId !== "local" || settings?.relayProfilesEnabled !== true) return false;
    if (!Array.isArray(settings.relayProfiles)) return false;
    const profile = settings.relayProfiles.find((item) => item?.id === settings.activeRelayId);
    if (!profile || profile.relayMode !== "official" || profile.officialMixApiKey !== true) return false;
    // openai 是会话身份，不一定是传输目标；混合模式可保留该身份并转发到 API。
    try {
      const url = new URL(profile.upstreamBaseUrl);
      if (!["http:", "https:"].includes(url.protocol)) return false;
      return !/(^|\.)openai\.com$|(^|\.)chatgpt\.com$/i.test(url.hostname);
    } catch {
      return false;
    }
  }

  function locate(text, url) {
    if (typeof text !== "string" || !text || typeof url !== "string") return null;
    const id = "[A-Za-z_$][\\w$]*";
    // 绑定身份校验和额度判断的特征，不能仅按压缩后的函数名定位。
    const atomPattern = new RegExp(`(${id})=${id}\\(${id},\\(\\{get:${id}\\}\\)=>\\{[^}]{0,1400}?\\.authMethod!==\`chatgpt\`[^}]{0,1400}?\\.rate_limit\\?\\.allowed!==!1`, "g");
    const atoms = [...text.matchAll(atomPattern)];
    if (atoms.length !== 1) return null;
    const escape = (value) => value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
    const gatePattern = new RegExp(`(${id})=${id}\\(${escape(atoms[0][1])}\\)&&(${id})===\`local\``, "g");
    const gates = [...text.matchAll(gatePattern)];
    if (gates.length !== 1) return null;
    const gate = gates[0];
    const tailStart = gate.index + gate[0].length;
    const tail = text.slice(tailStart, tailStart + 18000);
    const assignment = new RegExp(`(${id})=(${id}(?:\\|\\|${id})*\\|\\|${escape(gate[1])})(?=[,;])`).exec(tail);
    if (!assignment || !tail.includes(`submitDisabled:${assignment[1]}`)) return null;
    const offset = tailStart + assignment.index + assignment[1].length + 1;
    const before = text.slice(0, offset).split("\n");
    return {
      urlRegex: `^${escape(url)}$`,
      lineNumber: before.length - 1,
      columnNumber: before.at(-1).length,
      quotaVariable: gate[1],
      hostVariable: gate[2],
    };
  }

  function condition(location) {
    const identifier = /^[A-Za-z_$][\w$]*$/;
    if (!identifier.test(location?.quotaVariable) || !identifier.test(location?.hostVariable)) return null;
    const quota = location.quotaVariable;
    const host = location.hostVariable;
    // 条件始终返回 false，页面不会因这条断点暂停。
    return `(${quota}&&window.__codexPlusExternalApiQuotaAllowed?.(${host})===true&&(${quota}=false),false)`;
  }

  let refreshedComposers = new WeakMap();
  function refreshComposers(location, force = false) {
    if (!condition(location)) return 0;
    if (force) refreshedComposers = new WeakMap();
    const allowed = window.__codexPlusExternalApiQuotaAllowed?.("local") === true;
    let refreshed = 0;
    for (const root of document.querySelectorAll("[data-codex-composer-root]")) {
      let fiber = root[Object.keys(root).find(key => key.startsWith("__reactFiber$"))];
      for (let depth = 0; fiber && depth < 80; depth++, fiber = fiber.return) {
        if (typeof fiber.type !== "function") continue;
        const source = fiber.type.toString();
        if (!source.includes(`&&${location.hostVariable}===\`local\``)
          || !source.includes(`||${location.quotaVariable},`)
          || !source.includes("submitDisabled:")) continue;
        const hooks = [];
        for (let hook = fiber.memoizedState; hook; hook = hook.next) {
          if (hook.memoizedState instanceof Set && hook.memoizedState.size === 0
            && typeof hook.queue?.dispatch === "function") hooks.push(hook);
        }
        // 只重绘唯一可识别的空 Set 状态；不修改草稿或非空停止队列。
        if (hooks.length !== 1) break;
        const dispatch = hooks[0].queue.dispatch;
        const previous = refreshedComposers.get(dispatch);
        if (previous === allowed || (previous === undefined && !allowed && !force)) break;
        refreshedComposers.set(dispatch, allowed);
        dispatch(new Set());
        refreshed++;
        break;
      }
    }
    return refreshed;
  }

  const api = { permitsExternalApi, locate, condition, refreshComposers };
  if (typeof module === "object" && module.exports) module.exports = api;
  else window.__codexPlusApiQuotaGate = api;
})();
