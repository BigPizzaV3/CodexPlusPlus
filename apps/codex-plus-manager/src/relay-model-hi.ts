export type ModelHiTestOutcome<T> =
  | { model: string; status: "fulfilled"; value: T }
  | { model: string; status: "rejected"; reason: unknown };

type ModelHiTestHooks<T> = {
  onStart?: (model: string) => void;
  onSettled?: (outcome: ModelHiTestOutcome<T>) => void;
};

export function normalizeFetchedModelIds(fetchedModels: readonly string[]): string[] {
  return Array.from(new Set(fetchedModels.map((model) => model.trim()).filter(Boolean)));
}

export async function runAllFetchedModelHiTests<T>(
  fetchedModels: readonly string[],
  testModel: (model: string) => Promise<T>,
  hooks: ModelHiTestHooks<T> = {},
): Promise<ModelHiTestOutcome<T>[]> {
  const outcomes: ModelHiTestOutcome<T>[] = [];
  for (const model of normalizeFetchedModelIds(fetchedModels)) {
    hooks.onStart?.(model);
    let outcome: ModelHiTestOutcome<T>;
    try {
      outcome = { model, status: "fulfilled", value: await testModel(model) };
    } catch (reason) {
      outcome = { model, status: "rejected", reason };
    }
    outcomes.push(outcome);
    hooks.onSettled?.(outcome);
  }
  return outcomes;
}
