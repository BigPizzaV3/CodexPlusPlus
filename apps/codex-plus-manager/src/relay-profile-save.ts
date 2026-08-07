export async function persistRelayProfileDraft<T>({
  next,
  applyActiveProfile,
  saveSettings,
  shouldApplyActiveProfile,
}: {
  next: T;
  applyActiveProfile: (next: T) => Promise<void>;
  saveSettings: (next: T) => void | Promise<void>;
  shouldApplyActiveProfile: boolean;
}): Promise<void> {
  if (shouldApplyActiveProfile) {
    await applyActiveProfile(next);
    return;
  }
  await saveSettings(next);
}
