export const AUTOMATION_MODELS = [
  {
    label: "5.6 Sol",
    slug: "gpt-5.6-sol",
    defaultEffort: "low",
    efforts: ["low", "medium", "high", "xhigh", "max", "ultra"],
  },
  {
    label: "5.6 Terra",
    slug: "gpt-5.6-terra",
    defaultEffort: "medium",
    efforts: ["low", "medium", "high", "xhigh", "max", "ultra"],
  },
  {
    label: "5.6 Luna",
    slug: "gpt-5.6-luna",
    defaultEffort: "medium",
    efforts: ["low", "medium", "high", "xhigh", "max"],
  },
  {
    label: "5.5",
    slug: "gpt-5.5",
    defaultEffort: "medium",
    efforts: ["low", "medium", "high", "xhigh"],
  },
  {
    label: "5.4",
    slug: "gpt-5.4",
    defaultEffort: "medium",
    efforts: ["low", "medium", "high", "xhigh"],
  },
  {
    label: "5.4 Mini",
    slug: "gpt-5.4-mini",
    defaultEffort: "medium",
    efforts: ["low", "medium", "high", "xhigh"],
  },
];

const MODELS_BY_SLUG = new Map(AUTOMATION_MODELS.map((model) => [model.slug, model]));
const REASONING_EFFORTS = new Set(AUTOMATION_MODELS.flatMap((model) => model.efforts));

export function getAutomationModel(value) {
  return MODELS_BY_SLUG.get(value);
}

export function isAutomationModel(value) {
  return MODELS_BY_SLUG.has(value);
}

export function isAutomationReasoningEffort(value) {
  return REASONING_EFFORTS.has(value);
}

export function isSupportedModelEffort(model, effort) {
  return getAutomationModel(model)?.efforts.includes(effort) ?? false;
}

export function withAutomationModel(options, model) {
  const nextModel = getAutomationModel(model);
  return {
    ...options,
    model,
    reasoningEffort: nextModel.efforts.includes(options.reasoningEffort)
      ? options.reasoningEffort
      : nextModel.defaultEffort,
  };
}
