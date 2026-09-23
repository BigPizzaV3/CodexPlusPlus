export const DEFAULT_LABELS = [
  { name: "缺陷", color: "#eb5757" },
  { name: "特性", color: "#bb87fc" },
  { name: "for-claude", color: "#5b8cff" },
  { name: "hold", color: "#d99b25" },
  { name: "改进", color: "#4ea7fc" },
  { name: "phase-1", color: "#1d4ed8" },
  { name: "phase-2", color: "#0f766e" },
  { name: "phase-3", color: "#7c3aed" },
  { name: "phase-4", color: "#b45309" },
  { name: "phase-5", color: "#be123c" },
  { name: "phase-6", color: "#475569" },
] as const;

export function labelColor(name: string): string {
  return DEFAULT_LABELS.find((label) => label.name === name)?.color ?? "#8b8d92";
}
