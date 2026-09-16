// This is a local identification requirement, not an identity, rollout or approval result.
function cppNativeIdentificationReader(runtime, fallback, getTurnMetadata, controlPath) {
  return async function () {
    const info = this.clientInfo;
    const turn = getTurnMetadata(runtime);
    const sessionId = turn?.session_id;
    const turnId = turn?.turn_id;
    const instanceId = info?.metadata?.extensionInstanceId;
    const family = info?.family;
    const extensionId = info?.metadata?.extensionId;
    const supportedBrowser =
      (family === "edge" && extensionId === "odlomjlbamekndcpllcnffbgeohgkmjh") ||
      (family === "chrome" && extensionId === "hehggadaopoacecdllhhajmbjkdcmajg");
    if (info?.type !== "extension" || !supportedBrowser ||
        typeof info.agentRequestHeaderEnabled !== "boolean" ||
        typeof instanceId !== "string" || !instanceId ||
        typeof sessionId !== "string" || !sessionId ||
        typeof turnId !== "string" || !turnId) {
      return fallback();
    }
    let control;
    try {
      const fs = await import("node:fs/promises");
      const stat = await fs.lstat(controlPath);
      if (!stat.isFile() || stat.isSymbolicLink() || stat.size > 1024) throw new Error("Unsupported local control file");
      control = JSON.parse(await fs.readFile(controlPath, "utf8"));
    } catch {
      return fallback();
    }
    // Recheck after I/O; do not apply a decision to a replaced client or ended turn.
    const current = getTurnMetadata(runtime);
    if (this.clientInfo !== info || info.type !== "extension" || info.family !== family ||
        info.metadata?.extensionId !== extensionId ||
        info.metadata?.extensionInstanceId !== instanceId ||
        typeof info.agentRequestHeaderEnabled !== "boolean" ||
        current?.session_id !== sessionId || current?.turn_id !== turnId || control?.schema !== 1 ||
        control?.requireIdentification !== true) return fallback();
    return true;
  };
}
