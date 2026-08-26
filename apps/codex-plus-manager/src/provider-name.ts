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
