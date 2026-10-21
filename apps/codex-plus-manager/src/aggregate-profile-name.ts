/** 使用当前语言生成候选名称，并复用最小的未占用正整数编号。 */
export function nextAggregateProfileName(
  existingNames: readonly string[],
  formatName: (number: number) => string,
): string {
  const occupied = new Set(existingNames.map((name) => name.trim()));
  let number = 1;
  while (occupied.has(formatName(number).trim())) number += 1;
  return formatName(number);
}
