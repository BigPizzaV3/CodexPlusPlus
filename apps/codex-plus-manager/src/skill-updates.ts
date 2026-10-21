type SkillUpdateCandidate = {
  installed: boolean;
  bundled: boolean;
  plugin?: boolean;
  repoKey: string;
  updateAvailable: boolean;
  localDifference?: boolean;
  comparisonError?: string;
};

export function updatableSkills<T extends SkillUpdateCandidate>(skills: readonly T[]): T[] {
  return skills.filter((skill) => skill.installed && !skill.bundled && !skill.plugin
    && skill.repoKey.includes("/") && skill.updateAvailable
    && !skill.localDifference && !skill.comparisonError);
}
