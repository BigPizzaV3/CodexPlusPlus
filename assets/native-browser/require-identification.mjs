// This is a local identification requirement, not an identity, rollout or approval result.
function cppNativeIdentificationReader(runtime, fallback, getTurnMetadata, controlPath) {
  return async function () {
    const info = this.clientInfo;
    const turn = getTurnMetadata(runtime);
    if (info?.type !== "extension" || info.family !== "edge" ||
        info.metadata?.extensionId !== "odlomjlbamekndcpllcnffbgeohgkmjh" ||
        typeof info.agentRequestHeaderEnabled !== "boolean" ||
        typeof turn?.session_id !== "string" || !turn.session_id ||
        typeof turn?.turn_id !== "string" || !turn.turn_id) {
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
    if (this.clientInfo !== info || current?.session_id !== turn.session_id ||
        current?.turn_id !== turn.turn_id || control?.schema !== 1 ||
        control?.requireIdentification !== true) return fallback();
    return true;
  };
}
