export const zoomOptions = [90, 100, 110, 125, 150] as const;
export type DisplayDensity = "compact" | "standard" | "comfortable";
export type DisplayPreferences = { zoom: number; density: DisplayDensity };
const storageKey = "codex-plus-manager-display-v1";

export function normalizeDisplayPreferences(value: unknown): DisplayPreferences {
  const candidate = value as Partial<DisplayPreferences> | null;
  const zoom = zoomOptions.find((option) => option === candidate?.zoom) ?? 100;
  return {
    zoom,
    density: candidate?.density === "compact" || candidate?.density === "comfortable" ? candidate.density : "standard",
  };
}

export function loadDisplayPreferences(): DisplayPreferences {
  try { return normalizeDisplayPreferences(JSON.parse(localStorage.getItem(storageKey) || "null")); }
  catch { return normalizeDisplayPreferences(null); }
}

export function storeDisplayPreferences(value: DisplayPreferences) {
  localStorage.setItem(storageKey, JSON.stringify(value));
}
