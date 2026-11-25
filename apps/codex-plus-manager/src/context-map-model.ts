import type { ContextPart, ContextPartKind } from "./request-flow-model";

export type ContextMapKind = ContextPartKind | "unclassified" | "free";
export type ContextMapSlice = { kind: ContextMapKind; share: number };

export function contextMap(parts: ContextPart[], used: number | null, room: number | null) {
  const total = parts.reduce((sum, part) => sum + part.estimatedTokens, 0);
  const usedShare = room !== null && used !== null ? used / (used + room) : null;
  const occupied = usedShare ?? 1;
  const slices: ContextMapSlice[] = total > 0 && occupied > 0
    ? parts.filter(part => part.estimatedTokens > 0).map(part => ({ kind: part.kind, share: part.estimatedTokens / total * occupied }))
    : used && occupied > 0 ? [{ kind: "unclassified", share: occupied }] : [];
  if (usedShare !== null && usedShare < 1) slices.push({ kind: "free", share: 1 - usedShare });
  // Enlarge the occupied area for legibility; the grid rounds the real share to whole cells.
  const mapShare = usedShare !== null && usedShare > 0 && usedShare < 1 && total > 0
    ? Math.max(.34, Math.sqrt(usedShare)) : occupied;
  return { slices, usedShare, mapShare };
}

export function contextGrid(slices: ContextMapSlice[], size: number) {
  const used = slices.filter(slice => slice.kind !== "free");
  const total = used.reduce((sum, slice) => sum + slice.share, 0);
  const free = slices.find(slice => slice.kind === "free");
  const occupied = total > 0 ? Math.min(free && size > 1 ? size - 1 : size, Math.max(1, Math.round(total * size))) : 0;
  const quota = used.map(slice => slice.share / total * occupied);
  const counts = quota.map(Math.floor);
  const priority = quota.map((_, index) => index).sort((a, b) => (quota[b] - counts[b]) - (quota[a] - counts[a]));
  const spare = occupied - counts.reduce((sum, value) => sum + value, 0);
  for (const index of priority.slice(0, spare)) counts[index]++;
  return [...used.flatMap((slice, index) => Array<ContextMapKind>(counts[index]).fill(slice.kind)),
    ...(free ? Array<ContextMapKind>(size - occupied).fill("free") : [])];
}

export function contextCurve(ratios: (number | null)[], scale: number, width: number, baseline: number, top: number) {
  const points = ratios.flatMap((ratio, index) => ratio === null ? [] : [{
    index, x: (index + .5) * width / ratios.length, y: baseline - ratio / scale * (baseline - top),
  }]);
  const first = points[0];
  const last = points.at(-1);
  if (!first || !last || points.length < 2) return { area: "", observed: "", gaps: "" };
  let area = `M${first.x},${first.y}`;
  let observed = "";
  let gaps = "";
  for (let i = 1; i < points.length; i++) {
    const a = points[i - 1];
    const b = points[i];
    const mid = (a.x + b.x) / 2;
    const curve = `C${mid},${a.y} ${mid},${b.y} ${b.x},${b.y}`;
    area += curve;
    if (b.index === a.index + 1) observed += `M${a.x},${a.y}${curve}`;
    else gaps += `M${a.x},${a.y}${curve}`;
  }
  return { area: `${area}L${last.x},${baseline}H${first.x}Z`, observed, gaps };
}
