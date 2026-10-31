export type SkillSourceMatch = {
  localId: string; localName: string; localHash: string; snapshot: string;
  repoKey: string; repoPath: string; remoteName: string; remoteHash: string; exact: boolean;
};

export function sourceMatchKey(item: SkillSourceMatch): string {
  return JSON.stringify([item.repoKey, item.repoPath]);
}

function sourceMatchOriginKey(item: SkillSourceMatch): string {
  const repoPart = item.repoKey.split(":", 1)[0];
  const separator = repoPart.indexOf("@");
  const ownerRepo = separator < 0 ? repoPart : repoPart.slice(0, separator);
  const branch = separator < 0 ? "main" : repoPart.slice(separator + 1).trim() || "main";
  const parts = ownerRepo.split("/");
  const owner = parts[0]?.trim();
  const repo = parts[1]?.trim();
  if (parts.length !== 2 || !owner || !repo || !/^[a-z0-9._-]+$/i.test(owner) || !/^[a-z0-9._-]+$/i.test(repo)) {
    return JSON.stringify(["raw", item.repoKey, item.repoPath]);
  }
  return JSON.stringify(["github", owner.toLowerCase(), repo.toLowerCase(), branch, item.repoPath]);
}

export function sourceMatchLabel(item: SkillSourceMatch): string {
  return `${item.repoKey} / ${item.repoPath}`;
}

export function groupSourceMatches(items: SkillSourceMatch[]): Map<string, SkillSourceMatch[]> {
  const groups = new Map<string, SkillSourceMatch[]>();
  for (const item of items) {
    const group = groups.get(item.localId) || [];
    if (!group.some((existing) => sourceMatchKey(existing) === sourceMatchKey(item))) group.push(item);
    groups.set(item.localId, group);
  }
  for (const group of groups.values()) group.sort((a, b) => Number(b.exact) - Number(a.exact) || sourceMatchKey(a).localeCompare(sourceMatchKey(b)));
  return new Map([...groups].sort((a, b) => a[0].localeCompare(b[0])));
}

/** 只在候选唯一，或唯一的内容一致候选存在时预填选择；勾选仍须由用户完成。 */
export function preferredSourceMatch(items: SkillSourceMatch[]): string {
  const exact = items.filter((item) => item.exact);
  const chosen = exact.length === 1 ? exact[0] : items.length === 1 ? items[0] : undefined;
  return chosen ? sourceMatchKey(chosen) : "";
}

export function selectedSourceMatches(groups: Map<string, SkillSourceMatch[]>, selected: Record<string, string>): SkillSourceMatch[] | null {
  const result: SkillSourceMatch[] = [];
  const origins = new Set<string>();
  for (const [id, key] of Object.entries(selected)) {
    const item = groups.get(id)?.find((candidate) => sourceMatchKey(candidate) === key);
    if (!item) return null;
    const origin = sourceMatchOriginKey(item);
    if (origins.has(origin)) return null;
    origins.add(origin);
    result.push(item);
  }
  return result;
}
