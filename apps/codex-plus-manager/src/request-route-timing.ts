type RequestTiming = {
  durationMs: number;
  firstTokenMs?: number | null;
  stream?: boolean;
  state?: string | null;
};

export function requestRouteTiming(entry: RequestTiming) {
  const measured = (value: number | null | undefined) =>
    typeof value === "number" && Number.isFinite(value) && value >= 0 ? value : null;
  const total = entry.state === "running" ? null : measured(entry.durationMs);
  const candidate = entry.stream ? measured(entry.firstTokenMs) : null;
  const first = total != null && candidate != null && candidate > total ? null : candidate;
  return { total, first, afterFirst: total != null && first != null ? total - first : null };
}

export function requestTimingStages(entry: RequestTiming & {
  startedAtMs?: number;
  route?: { finishedAtMs: number; elapsedMs: number; steps: { outcome: string; elapsedMs: number }[] } | null;
}) {
  const { total, first } = requestRouteTiming(entry);
  const route = entry.route;
  const accepted = route?.steps.filter(step => step.outcome === "answered").at(-1)?.elapsedMs;
  const before = route && entry.startedAtMs ? route.finishedAtMs - route.elapsedMs - entry.startedAtMs : null;
  const aligned = route && before !== null && Number.isFinite(before) && before >= 0 && accepted != null
    && Number.isFinite(accepted) && accepted >= 0 && accepted <= route.elapsedMs ? before + accepted : null;
  if (aligned !== null && before !== null && accepted != null && total !== null && aligned <= (first ?? total)) {
    return [
      { key: "prepare", ms: before }, { key: "route", ms: accepted },
      ...(first !== null ? [{ key: "body", ms: first - aligned }, { key: "transfer", ms: total - first }]
        : [{ key: "finish", ms: total - aligned }]),
    ];
  }
  if (first !== null) return [{ key: "first", ms: first }, ...(total !== null ? [{ key: "transfer", ms: total - first }] : [])];
  return [];
}
