export type GroupableSkill = { id: string; repoKey: string; bundled: boolean };
export type GroupableRepository = { owner: string; name: string };
export type SkillGroup<T> = { key: string; label: string; kind: "builtin" | "user" | "repo" | "custom"; items: T[] };
export type SkillGroupSortDirection = "asc" | "desc";
type SkillGroupStorage = Record<string, unknown> & {
  names: string[];
  assignments: Record<string, string>;
  assignmentsByIdentity: Record<string, string>;
  order: string[];
  sortDirection: SkillGroupSortDirection | null;
  aliases: Record<string, string>;
};

export const BUILTIN_SKILL_GROUP = "builtin";
export const USER_SKILL_GROUP = "user";
export const customSkillGroupKey = (name: string) => `custom:${name}`;
export const repositorySkillGroupKey = (name: string) => `repo:${name}`;
export const skillIdentityKey = (skill: Pick<GroupableSkill, "repoKey" | "id">) => JSON.stringify([skill.repoKey, skill.id]);

/** 导入渠道不是仓库；只有明确的 owner/repo@branch 来源才创建仓库分组。 */
export function skillRepositoryName(repoKey: string): string | null {
  const match = /^([^/@:]+\/[^/@:]+)@[^:]+(?::.*)?$/.exec(repoKey);
  return match?.[1] ?? null;
}

export function defaultSkillGroupKey(skill: GroupableSkill): string {
  if (skill.bundled) return BUILTIN_SKILL_GROUP;
  const repository = skillRepositoryName(skill.repoKey);
  return repository ? repositorySkillGroupKey(repository) : USER_SKILL_GROUP;
}

export function legacySkillCustomGroupName(value: string, customNames: string[]): string | null {
  if (customNames.includes(value)) return value;
  const name = value.startsWith("custom:") ? value.slice(7) : "";
  return customNames.includes(name) ? name : null;
}

export function skillGroupAssignment(
  skill: GroupableSkill,
  customNames: string[],
  assignments: Record<string, string>,
  assignmentsByIdentity: Record<string, string>,
): string {
  const key = skillIdentityKey(skill);
  // 空字符串表示主动选择按来源，不能再回退到旧的 ID 分配。
  if (Object.prototype.hasOwnProperty.call(assignmentsByIdentity, key)) return assignmentsByIdentity[key];
  const legacy = Object.prototype.hasOwnProperty.call(assignments, skill.id) ? assignments[skill.id] : "";
  const customName = legacySkillCustomGroupName(legacy, customNames);
  return customName === null ? legacy : customSkillGroupKey(customName);
}

export function readSkillGroupStorage(value: string | null): SkillGroupStorage {
  let stored: Record<string, unknown> = {};
  try {
    const parsed = JSON.parse(value || "{}");
    if (parsed && typeof parsed === "object" && !Array.isArray(parsed)) stored = parsed;
  } catch {}
  const stringRecord = (input: unknown): Record<string, string> => input && typeof input === "object" && !Array.isArray(input)
    ? Object.fromEntries(Object.entries(input).filter((entry): entry is [string, string] => typeof entry[1] === "string")) : {};
  const sortDirection: SkillGroupSortDirection | null = stored.sortDirection === "asc" || stored.sortDirection === "desc" ? stored.sortDirection : null;
  return {
    ...stored,
    names: Array.isArray(stored.names) ? stored.names.filter((name): name is string => typeof name === "string" && name.trim().length > 0) : [],
    assignments: stringRecord(stored.assignments),
    assignmentsByIdentity: stringRecord(stored.assignmentsByIdentity),
    order: Array.isArray(stored.order) ? stored.order.filter((key): key is string => typeof key === "string") : [],
    sortDirection,
    aliases: stringRecord(stored.aliases),
  };
}

export function groupSkills<T extends GroupableSkill>(
  visible: T[],
  known: T[],
  customNames: string[],
  assignments: Record<string, string>,
  order: string[],
  labels: { builtin: string; user: string; source: (skill: T) => string },
  includeEmpty = true,
  aliases: Record<string, string> = {},
  assignmentsByIdentity: Record<string, string> = {},
  knownRepositories: GroupableRepository[] = [],
): SkillGroup<T>[] {
  const groups = new Map<string, SkillGroup<T>>();
  const sourceLabels = new Map(known.map((skill) => [defaultSkillGroupKey(skill), labels.source(skill)]));
  const add = (key: string, label: string, kind: SkillGroup<T>["kind"]) => {
    if (!groups.has(key)) groups.set(key, { key, label, kind, items: [] });
  };
  if (includeEmpty) {
    add(BUILTIN_SKILL_GROUP, labels.builtin, "builtin");
    add(USER_SKILL_GROUP, labels.user, "user");
    for (const name of customNames) add(customSkillGroupKey(name), name, "custom");
    for (const skill of known) {
      const name = !skill.bundled && skillRepositoryName(skill.repoKey);
      if (name) {
        const key = repositorySkillGroupKey(name);
        add(key, aliases[key] || labels.source(skill) || name, "repo");
      }
    }
    // 与后端仓库配置的 owner/name 规则一致，导入渠道本身不创建仓库组。
    for (const repo of knownRepositories) {
      if (!/^[A-Za-z0-9._-]+$/.test(repo.owner) || !/^[A-Za-z0-9._-]+$/.test(repo.name)) continue;
      const name = `${repo.owner}/${repo.name}`;
      const key = repositorySkillGroupKey(name);
      add(key, aliases[key] || name, "repo");
    }
  }
  for (const skill of visible) {
    const assignment = skillGroupAssignment(skill, customNames, assignments, assignmentsByIdentity);
    let key = defaultSkillGroupKey(skill);
    if (!skill.bundled && assignment === USER_SKILL_GROUP) key = USER_SKILL_GROUP;
    else if (!skill.bundled && assignment.startsWith("custom:") && customNames.includes(assignment.slice(7))) key = assignment;
    else if (!skill.bundled && assignment.startsWith("repo:") && assignment.length > 5) key = assignment;
    if (key === BUILTIN_SKILL_GROUP) add(key, labels.builtin, "builtin");
    else if (key === USER_SKILL_GROUP) add(key, labels.user, "user");
    else if (key.startsWith("custom:")) add(key, key.slice(7), "custom");
    else add(key, aliases[key] || sourceLabels.get(key) || key.slice(5), "repo");
    groups.get(key)!.items.push(skill);
  }
  const position = new Map(order.map((key, index) => [key, index]));
  return [...groups.values()].sort((first, second) => {
    const pinned = (group: SkillGroup<T>) => group.kind === "builtin" ? 0 : group.kind === "user" ? 1 : 2;
    const rank = pinned(first) - pinned(second);
    if (rank) return rank;
    const firstPosition = position.get(first.key) ?? Number.MAX_SAFE_INTEGER;
    const secondPosition = position.get(second.key) ?? Number.MAX_SAFE_INTEGER;
    return firstPosition - secondPosition || first.label.localeCompare(second.label);
  });
}

export function sortSkillGroupsByName<T>(groups: SkillGroup<T>[], direction: SkillGroupSortDirection): SkillGroup<T>[] {
  const pinned = groups.filter((group) => group.kind === "builtin" || group.kind === "user");
  const movable = groups.filter((group) => group.kind === "repo" || group.kind === "custom")
    .sort((first, second) => (direction === "asc" ? 1 : -1) * first.label.localeCompare(second.label));
  return [...pinned, ...movable];
}

export function sortSkillGroupKeys<T>(groups: SkillGroup<T>[], direction: SkillGroupSortDirection): string[] {
  return sortSkillGroupsByName(groups, direction)
    .filter((group) => group.kind === "repo" || group.kind === "custom").map((group) => group.key);
}

export function nextSkillGroupSortDirection(direction: SkillGroupSortDirection | null): SkillGroupSortDirection {
  return direction === "asc" ? "desc" : "asc";
}

export function moveSkillGroup<T>(groups: SkillGroup<T>[], from: string, to: string): string[] {
  const movable = groups.filter((group) => group.kind === "repo" || group.kind === "custom").map((group) => group.key);
  const source = movable.indexOf(from);
  const destination = movable.indexOf(to);
  if (source < 0 || destination < 0 || source === destination) return movable;
  movable.splice(destination, 0, ...movable.splice(source, 1));
  return movable;
}
