(() => {
  // 本机官登只要勾了混入 Key，就解除官方订阅额度的发送锁。不看上游地址。
  function permitsExternalApi(settings, hostId) {
    if (hostId !== "local" || settings?.relayProfilesEnabled !== true) return false;
    if (!Array.isArray(settings.relayProfiles)) return false;
    const profile = settings.relayProfiles.find((item) => item?.id === settings.activeRelayId);
    return !!profile && profile.relayMode === "official" && profile.officialMixApiKey === true;
  }

  const api = { permitsExternalApi };
  if (typeof window !== "undefined") window.__codexPlusApiQuotaGate = api;
  if (typeof module === "object" && module.exports) module.exports = api;
})();
