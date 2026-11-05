export type SubscriptionPolicy = {
  defaultUsedPercentLimit?: number | null;
  windowUsedPercentLimits: Record<string, number>;
  onLimit: "stop" | "nextPermitted" | "apiFallback";
  unknownUsage: "block" | "allow";
  maxAgeSeconds: number;
};

/** 后端省略空窗口表和未设置的阈值；旧记录可能没有新增字段。 */
export type SubscriptionPolicyInput = Partial<Omit<SubscriptionPolicy, "windowUsedPercentLimits">> & {
  windowUsedPercentLimits?: Record<string, number> | null;
};

export function defaultSubscriptionPolicy(): SubscriptionPolicy {
  return {
    defaultUsedPercentLimit: null,
    windowUsedPercentLimits: {},
    onLimit: "stop",
    unknownUsage: "allow",
    maxAgeSeconds: 600,
  };
}

export function normalizeSubscriptionPolicy(value?: SubscriptionPolicyInput | null): SubscriptionPolicy {
  const defaults = defaultSubscriptionPolicy();
  return {
    defaultUsedPercentLimit: value?.defaultUsedPercentLimit ?? defaults.defaultUsedPercentLimit,
    windowUsedPercentLimits: { ...value?.windowUsedPercentLimits },
    onLimit: value?.onLimit ?? defaults.onLimit,
    unknownUsage: value?.unknownUsage ?? defaults.unknownUsage,
    maxAgeSeconds: value?.maxAgeSeconds ?? defaults.maxAgeSeconds,
  };
}

/** 官方累计已用整数比例；100/留空清除阈值，undefined 表示输入无效。 */
export function subscriptionPercentLimit(text: string): number | null | undefined {
  if (!text.trim()) return null;
  const number = Number(text);
  if (!Number.isInteger(number) || number < 0 || number > 100) return undefined;
  return number === 100 ? null : number;
}

export function withSubscriptionWindowLimit(policy: SubscriptionPolicy, minutes: string, text: string): SubscriptionPolicy {
  const windowUsedPercentLimits = { ...policy.windowUsedPercentLimits };
  const limit = subscriptionPercentLimit(text);
  if (limit === undefined) return policy;
  if (limit === null) delete windowUsedPercentLimits[minutes];
  else windowUsedPercentLimits[minutes] = limit;
  return { ...policy, windowUsedPercentLimits };
}
