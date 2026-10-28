/** ⌘K 命令面板的数据与过滤逻辑（不依赖 React，便于测试）。 */
import type { ReactNode } from "react";

export type PaletteCommand = {
  id: string;
  /** 分组标题，例如「网关」「Codex」「操作」。 */
  group: string;
  label: string;
  /** 额外的匹配词（英文名、路由 id 等），不显示。 */
  keywords?: string;
  icon?: ReactNode;
  run: () => void;
};

/** 按空格切成多个词，每个词都要出现在标签、分组或关键词里。 */
export function filterCommands(commands: PaletteCommand[], query: string): PaletteCommand[] {
  const terms = query.trim().toLowerCase().split(/\s+/).filter(Boolean);
  if (!terms.length) return commands;
  return commands.filter((command) => {
    const haystack = `${command.label} ${command.group} ${command.keywords ?? ""}`.toLowerCase();
    return terms.every((term) => haystack.includes(term));
  });
}
