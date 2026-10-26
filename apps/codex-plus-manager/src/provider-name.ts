// 读取 config.toml 中 [model_providers.<id>] 表内已有的 name 值。
//
// codex 允许 provider 表名与其 name 字段不同（name 只是展示用的标签），
// 用户可能特意把 name 改成 "OpenAi" 之类。写默认值前先查一次，
// 避免把用户改过的 name 覆盖回表名（issue #1886）。
export function tomlSectionStringValue(contents: string, sectionName: string, key: string): string {
  const lines = contents.split(/\r?\n/);
  let inSection = false;
  const keyPattern = new RegExp(`^\\s*${key}\\s*=\\s*(["'])(.*)\\1\\s*(?:#.*)?$`);
  for (const line of lines) {
    const section = /^\s*\[([^\]]+)\]\s*$/.exec(line);
    if (section) {
      inSection = section[1].trim() === sectionName;
      continue;
    }
    if (!inSection) continue;
    const match = keyPattern.exec(line.trim());
    if (match) return match[2].replace(/\\(["'\\])/g, "$1");
  }
  return "";
}

// provider 表缺 name 时该填什么：已有非空 name 就沿用，否则回落到表名。
export function resolveProviderName(contents: string, provider: string): string {
  const existing = tomlSectionStringValue(contents, `model_providers.${provider}`, "name").trim();
  return existing || provider;
}

// 供应商头像上的两个字。规则：先取第一个字；第二个字优先取「另一种类型」的字
// （中文 / 英文字母 / 数字之间互为不同类型），例如「哈基米 国模 kimi」→「哈K」、「供应商 4」→「供4」。
// 名字里只有一种类型时：多个词取第二个词的首字（「Max JOJO Code」→「MJ」、「哈基米 国模」→「哈国」）；
// 单个英文词取驼峰的下一个大写字母（「DeepSeek」→「DS」），否则取第二个字。
type AvatarCharKind = "han" | "latin" | "digit";

function avatarCharKind(char: string): AvatarCharKind | null {
  if (/\p{Script=Han}/u.test(char)) return "han";
  if (/[A-Za-z]/.test(char)) return "latin";
  if (/[0-9]/.test(char)) return "digit";
  return null;
}

export function providerAvatarText(name: string, fallback = "供"): string {
  const chars = Array.from(name.trim());
  const meaningful = chars
    .map((char, index) => ({ char, index, kind: avatarCharKind(char) }))
    .filter((item): item is { char: string; index: number; kind: AvatarCharKind } => item.kind !== null);
  if (!meaningful.length) return fallback;
  const first = meaningful[0];
  const upper = (char: string) => char.toUpperCase();
  const otherKind = meaningful.find((item) => item.kind !== first.kind);
  if (otherKind) return upper(first.char) + upper(otherKind.char);
  // 只有一种类型：按词切分（空白、括号、标点都算分隔）。
  const words = name.trim().split(/[\s()（）\[\]【】,，.。·_\-/|]+/u).filter((word) => Array.from(word).some((char) => avatarCharKind(char)));
  if (words.length >= 2) {
    const second = Array.from(words[1]).find((char) => avatarCharKind(char));
    if (second) return upper(first.char) + upper(second);
  }
  if (first.kind === "latin") {
    const camel = meaningful.slice(1).find((item) => /[A-Z]/.test(item.char));
    if (camel) return upper(first.char) + camel.char;
  }
  const second = meaningful[1];
  return second ? upper(first.char) + upper(second.char) : upper(first.char);
}
