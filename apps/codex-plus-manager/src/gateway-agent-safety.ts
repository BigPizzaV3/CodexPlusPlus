import type { GatewayAgentView, GatewayDisconnectPreview } from "./gateway-model";

export type AgentDiagnosticPresentation = {
  badge: string;
  tone: "on" | "error" | "off" | "pending";
  reconnect: boolean;
};

/** 关闭旧版供应商写入开关时，Codex 的重新接入必定失败；旧版 payload 未知时保持原行为。 */
export function canReconnectAgent(agent: Pick<GatewayAgentView, "agent">, relayProfilesEnabled?: boolean): boolean {
  return agent.agent !== "codex" || relayProfilesEnabled !== false;
}

/** 配置已写入和请求已到达是两种证据，不能把没有流量当成绕过网关或接入成功。 */
export function agentDiagnosticPresentation(agent: GatewayAgentView, helperRunning: boolean): AgentDiagnosticPresentation {
  if (!agent.enabled) return { badge: "未接入", tone: "off", reconnect: false };
  const state = agent.diagnostics?.state;
  if (state === "configUnreadable") return { badge: "配置无法读取", tone: "error", reconnect: false };
  if (state === "configUnverified") return { badge: "配置无法验证", tone: "error", reconnect: false };
  if (state === "configMissing") return { badge: "配置缺失", tone: "error", reconnect: true };
  if (state === "configChanged" || (!state && agent.drift)) return { badge: "配置被改动", tone: "error", reconnect: true };
  if (!helperRunning || state === "gatewayStopped") return { badge: "网关未运行", tone: "error", reconnect: false };
  if (state === "upstreamError") return { badge: "上游请求失败", tone: "error", reconnect: false };
  if (state === "active") return { badge: "请求已到达", tone: "on", reconnect: false };
  if (state === "disabled") return { badge: "未接入", tone: "off", reconnect: false };
  if (state && state !== "awaitingTraffic") return { badge: "状态待确认", tone: "pending", reconnect: false };
  return { badge: "等待请求验证", tone: "pending", reconnect: false };
}

export function disconnectPreviewReady(preview: GatewayDisconnectPreview | null, agent: string): preview is GatewayDisconnectPreview {
  return Boolean(preview?.canDisconnect && preview.agent === agent && preview.previewToken.trim());
}

/** 预览没法逐项列出改动、但可以完整切换到某个直连供应商：给「完整切换并断开」。 */
export function fullSwitchReady(preview: GatewayDisconnectPreview | null, agent: string): preview is GatewayDisconnectPreview {
  return Boolean(preview && !preview.canDisconnect && preview.fullSwitch && preview.agent === agent && preview.previewToken.trim());
}

export function disconnectFileLabel(kind: GatewayDisconnectPreview["files"][number]["kind"]): string {
  switch (kind) {
    case "create": return "将创建";
    case "remove": return "将移除";
    case "modify": return "将修改";
  }
}

export function diagnosticEvidenceLabel(code: string): string {
  const labels: Record<string, string> = {
    matches: "与网关配置一致",
    matched: "与网关配置一致",
    missing: "配置项缺失",
    missingFile: "文件不存在",
    missing_file: "文件不存在",
    changed: "配置项已改变",
    mismatch: "与网关配置不符",
    endpointChanged: "地址已改变",
    endpoint_changed: "地址已改变",
    unreadable: "无法读取",
    parseError: "无法解析",
    parse_error: "无法解析",
    credentialChanged: "本地凭据已改变",
    credential_changed: "本地凭据已改变",
    credentialMissing: "本地凭据缺失",
    credential_missing: "本地凭据缺失",
    endpointMatched: "与网关配置一致",
    endpointPortMismatch: "端口与网关配置不符",
    endpointHostMismatch: "主机与网关配置不符",
    endpointPathMismatch: "路径与网关配置不符",
    endpointMissing: "地址缺失",
    selectionChanged: "当前供应商或模型已改变",
    fileMissing: "文件不存在",
    fileUnreadable: "无法读取",
    invalidConfig: "配置格式无效",
    unsupportedConfig: "此配置暂无法验证",
    endpoint_match: "与网关配置一致",
    credential_match: "与网关配置一致",
    selection_match: "与网关配置一致",
    managed_config_match: "与网关配置一致",
    config_missing: "配置缺失",
    config_unreadable: "无法读取",
    config_invalid: "配置格式无效",
    config_unverified: "此配置暂无法验证",
    unsupported_agent: "此 Agent 暂无法验证",
    endpoint_missing: "地址缺失",
    endpoint_invalid: "地址格式无效",
    endpoint_host_mismatch: "主机与网关配置不符",
    endpoint_port_mismatch: "端口与网关配置不符",
    endpoint_path_mismatch: "路径与网关配置不符",
    selection_changed: "当前供应商或模型已改变",
    managed_config_changed: "托管配置已改变",
  };
  return labels[code] ?? "诊断信息待确认";
}

export function diagnosticErrorLabel(kind: string): string {
  const labels: Record<string, string> = {
    authInvalid: "认证已失效",
    rateLimited: "请求频率受限",
    quotaExhausted: "可用额度不足",
    serverError: "上游服务异常",
    clientError: "上游拒绝请求",
    transport: "上游连接失败",
    cooling: "上游暂时冷却",
    upstreamError: "上游请求失败",
    auth_invalid: "认证已失效",
    rate_limited: "请求频率受限",
    quota_exhausted: "可用额度不足",
    server_error: "上游服务异常",
    client_error: "上游拒绝请求",
    upstream_error: "上游请求失败",
  };
  return labels[kind] ?? "错误原因待确认";
}
