/**
 * 设置在管理器之外被改了（Codex 设置页、网关接入）时，把管理器里尚未保存的编辑叠到最新设置上。
 *
 * - 没有本地编辑：结果就是最新设置，界面立即反映外部改动；
 * - 有本地编辑：只保留与上次读到的值不同的字段，其余取最新值；
 * - revision 一律取最新，之后的自动保存不会撞上 settings_revision_conflict。
 *
 * 三个参数应先经过同一套规范化，否则规范化差异会被误判成本地编辑。
 */
export function rebaseSettingsEdits<T extends object>(form: T, previous: T, latest: T): T {
  const merged = { ...latest } as Record<string, unknown>;
  const local = form as Record<string, unknown>;
  const base = previous as Record<string, unknown>;
  for (const key of Object.keys(local)) {
    if (key === "revision") continue;
    if (JSON.stringify(local[key]) !== JSON.stringify(base[key])) merged[key] = local[key];
  }
  return merged as T;
}
