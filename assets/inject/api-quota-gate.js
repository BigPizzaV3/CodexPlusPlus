(() => {
  // 只判断本机官登混合模式是否把普通请求转发到外部中转。
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

  const api = { permitsExternalApi };
  if (typeof window !== "undefined") window.__codexPlusApiQuotaGate = api;
  if (typeof module === "object" && module.exports) module.exports = api;
})();
