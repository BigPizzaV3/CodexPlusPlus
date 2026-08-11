export type CodexGoalsFeatureState = {
  enabled: boolean;
  inherited: boolean;
};

export function codexGoalsFeatureValue(configContents: string): boolean | undefined {
  let inFeatures = false;
  for (const line of configContents.split(/\r?\n/)) {
    const trimmed = line.trim();
    if (/^\[features\]$/.test(trimmed)) {
      inFeatures = true;
      continue;
    }
    if (inFeatures && /^\[[^\]]+\]$/.test(trimmed)) {
      inFeatures = false;
    }
    if (inFeatures) {
      const match = /^goals\s*=\s*(true|false)\b/.exec(trimmed);
      if (match) return match[1] === "true";
    }
  }
  return undefined;
}

export function codexGoalsFeatureState(
  profileConfigContents: string,
  commonConfigContents: string,
  useCommonConfig: boolean,
): CodexGoalsFeatureState {
  const profileValue = codexGoalsFeatureValue(profileConfigContents);
  if (profileValue !== undefined) {
    return { enabled: profileValue, inherited: false };
  }
  const commonValue = useCommonConfig ? codexGoalsFeatureValue(commonConfigContents) : undefined;
  if (commonValue !== undefined) {
    return { enabled: commonValue, inherited: true };
  }
  return { enabled: false, inherited: false };
}

export function setCodexGoalsFeatureInConfig(configContents: string, enabled: boolean): string {
  const lines = configContents.split(/\r?\n/);
  const next: string[] = [];
  let inFeatures = false;
  let sawFeatures = false;
  let featuresHasGoals = false;
  let featuresHeaderIndex = -1;

  const maybeInsertGoals = () => {
    if (sawFeatures && !featuresHasGoals) {
      next.splice(featuresHeaderIndex + 1, 0, `goals = ${enabled ? "true" : "false"}`);
      featuresHasGoals = true;
    }
  };

  for (const line of lines) {
    const trimmed = line.trim();
    if (/^\[features\]$/.test(trimmed)) {
      if (inFeatures) maybeInsertGoals();
      inFeatures = true;
      sawFeatures = true;
      featuresHasGoals = false;
      next.push(line);
      featuresHeaderIndex = next.length - 1;
      continue;
    }
    if (inFeatures && /^\[[^\]]+\]$/.test(trimmed)) {
      maybeInsertGoals();
      inFeatures = false;
    }
    if (inFeatures && /^goals\s*=/.test(trimmed)) {
      if (!featuresHasGoals) {
        next.push(`goals = ${enabled ? "true" : "false"}`);
        featuresHasGoals = true;
      }
      continue;
    }
    next.push(line);
  }

  if (inFeatures) maybeInsertGoals();
  if (!sawFeatures) {
    const trimmed = ensureTrailingNewline(next.join("\n").trimEnd());
    return joinTomlSections([trimmed, `[features]\ngoals = ${enabled ? "true" : "false"}`]);
  }

  return ensureTrailingNewline(next.join("\n").trimEnd());
}

function ensureTrailingNewline(value: string): string {
  return value.endsWith("\n") ? value : `${value}\n`;
}

function joinTomlSections(sections: string[]): string {
  const normalized = sections.map((section) => section.trim()).filter(Boolean);
  return normalized.length ? `${normalized.join("\n\n")}\n` : "";
}
