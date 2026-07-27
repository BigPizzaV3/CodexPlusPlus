export function configHasFeature(configContents: string, feature: string): boolean {
  let inFeatures = false;
  const featurePattern = new RegExp(`^${escapeRegExp(feature)}\\s*=\\s*true\\b`);

  for (const line of configContents.split(/\r?\n/)) {
    const trimmed = line.trim();
    if (/^\[features\]$/.test(trimmed)) {
      inFeatures = true;
      continue;
    }
    if (inFeatures && /^\[[^\]]+\]$/.test(trimmed)) {
      inFeatures = false;
    }
    if (inFeatures && featurePattern.test(trimmed)) return true;
  }
  return false;
}

export function setFeatureInConfig(configContents: string, feature: string, enabled: boolean): string {
  const lines = configContents.split(/\r?\n/);
  const next: string[] = [];
  const featurePattern = new RegExp(`^${escapeRegExp(feature)}\\s*=`);
  let inFeatures = false;
  let sawFeatures = false;
  let featuresHasFeature = false;

  const maybeInsertFeature = () => {
    if (enabled && sawFeatures && !featuresHasFeature) {
      next.push(`${feature} = true`);
      featuresHasFeature = true;
    }
  };

  for (const line of lines) {
    const trimmed = line.trim();
    if (/^\[features\]$/.test(trimmed)) {
      if (inFeatures) maybeInsertFeature();
      inFeatures = true;
      sawFeatures = true;
      featuresHasFeature = false;
      next.push(line);
      continue;
    }
    if (inFeatures && /^\[[^\]]+\]$/.test(trimmed)) {
      maybeInsertFeature();
      inFeatures = false;
    }
    if (inFeatures && featurePattern.test(trimmed)) {
      if (enabled && !featuresHasFeature) {
        next.push(`${feature} = true`);
        featuresHasFeature = true;
      }
      continue;
    }
    next.push(line);
  }

  if (inFeatures) maybeInsertFeature();
  if (enabled && !sawFeatures) {
    return joinTomlSections([ensureTrailingNewline(next.join("\n").trimEnd()), `[features]\n${feature} = true`]);
  }
  return ensureTrailingNewline(next.join("\n").trimEnd());
}

function escapeRegExp(value: string): string {
  return value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

function ensureTrailingNewline(value: string): string {
  return value.trim() ? `${value}\n` : "";
}

function joinTomlSections(sections: string[]): string {
  return ensureTrailingNewline(sections.map((section) => section.trim()).filter(Boolean).join("\n\n"));
}
