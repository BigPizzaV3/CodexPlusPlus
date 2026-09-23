export function getRevisionPollingInterval(metadata) {
  if (metadata?.mode !== "cloud" || metadata.realtime?.transport !== "poll") return null;
  return metadata.realtime.intervalMs;
}

export function createRevisionPoller({
  fetchRevision,
  onInvalidate,
  intervalMs = 2000,
  setInterval: scheduleInterval = globalThis.setInterval,
  clearInterval: cancelInterval = globalThis.clearInterval,
}) {
  let revision = 0;
  let timer;
  let running = false;
  let requestPending = false;

  async function poll() {
    if (!running || requestPending) return;
    requestPending = true;
    try {
      const result = await fetchRevision(revision);
      if (!running) return;
      const advanced = result.revision > revision;
      revision = Math.max(revision, result.revision);
      if (result.changed && advanced) onInvalidate();
    } catch {
      // Keep the current revision so the next interval retries the same boundary.
    } finally {
      requestPending = false;
    }
  }

  return {
    start() {
      if (running) return;
      running = true;
      timer = scheduleInterval(poll, intervalMs);
      void poll();
    },
    stop() {
      if (!running) return;
      running = false;
      cancelInterval(timer);
      timer = undefined;
    },
  };
}
