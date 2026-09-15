export type AutomationModel =
  | "gpt-5.6-sol"
  | "gpt-5.6-terra"
  | "gpt-5.6-luna"
  | "gpt-5.5"
  | "gpt-5.4"
  | "gpt-5.4-mini";

export type AutomationReasoningEffort =
  | "low"
  | "medium"
  | "high"
  | "xhigh"
  | "max"
  | "ultra";

export interface AutomationModelOption {
  readonly label: string;
  readonly slug: AutomationModel;
  readonly defaultEffort: AutomationReasoningEffort;
  readonly efforts: readonly AutomationReasoningEffort[];
}

export const AUTOMATION_MODELS: readonly AutomationModelOption[];

export function getAutomationModel(value: AutomationModel): AutomationModelOption;
export function getAutomationModel(value: unknown): AutomationModelOption | undefined;
export function isAutomationModel(value: unknown): value is AutomationModel;
export function isAutomationReasoningEffort(
  value: unknown,
): value is AutomationReasoningEffort;
export function isSupportedModelEffort(
  model: unknown,
  effort: unknown,
): model is AutomationModel;
export function withAutomationModel<
  T extends { model: AutomationModel; reasoningEffort: AutomationReasoningEffort },
>(
  options: T,
  model: AutomationModel,
): Omit<T, "model" | "reasoningEffort"> & {
  model: AutomationModel;
  reasoningEffort: AutomationReasoningEffort;
};
