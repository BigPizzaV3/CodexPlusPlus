export type ProviderSyncSelectableTarget = {
  id: string;
  isCurrentProvider: boolean;
  isResolvable: boolean;
};

export function isProviderSyncTargetSelectable(target: ProviderSyncSelectableTarget): boolean {
  return target.isResolvable === true;
}

export function preferredProviderSyncTarget(
  targets: readonly ProviderSyncSelectableTarget[],
  currentProvider: string,
  savedProvider: string,
): string {
  return (
    targets.find((target) => target.id === currentProvider && isProviderSyncTargetSelectable(target))?.id ??
    targets.find((target) => target.isCurrentProvider && isProviderSyncTargetSelectable(target))?.id ??
    targets.find((target) => target.id === savedProvider && isProviderSyncTargetSelectable(target))?.id ??
    targets.find(isProviderSyncTargetSelectable)?.id ??
    ""
  );
}
