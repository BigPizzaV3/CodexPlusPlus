import { useState } from "react";
import type { ProviderPreset, RelayProtocol } from "../presets";
import { PRESETS } from "../presets";

export type RelayProfile = {
  id: string;
  linkedCcsProviderId: string;
  name: string;
  model: string;
  baseUrl: string;
  upstreamBaseUrl: string;
  apiKey: string;
  protocol: RelayProtocol;
  relayMode: string;
  officialMixApiKey: boolean;
  testModel: string;
  configContents: string;
  authContents: string;
  useCommonConfig: boolean;
  contextWindow: string;
  autoCompactLimit: string;
  modelInsertMode: string;
  modelList: string;
  userAgent: string;
};

export type PresetPatch = Partial<RelayProfile>;

const categoryLabels: Record<string, string> = {
  official: "官方",
  cn_official: "中国官方",
  aggregator: "聚合/中转",
  third_party: "第三方",
};

export function createPresetPatch(preset: ProviderPreset): PresetPatch {
  return {
    name: preset.name,
    baseUrl: preset.baseUrl,
    upstreamBaseUrl: preset.baseUrl,
    protocol: preset.protocol,
    model: preset.model,
    testModel: preset.model,
    modelList: preset.modelList?.join("\n") ?? "",
    relayMode: preset.category === "official" ? "official" : "pureApi",
    officialMixApiKey: false,
  };
}

export function ProviderPresetSelector({
  onSelect,
}: {
  onSelect: (patch: PresetPatch) => void;
}) {
  const [collapsed, setCollapsed] = useState(true);

  const categories = [...new Set(PRESETS.map((p) => p.category))];

  return (
    <div className="preset-selector">
      <button
        className="preset-toggle"
        aria-expanded={!collapsed}
        onClick={() => setCollapsed((c) => !c)}
        type="button"
      >
        <span>{collapsed ? "从预设模板创建" : "收起预设模板"}</span>
        <small>{collapsed ? `共 ${PRESETS.length} 个供应商` : ""}</small>
      </button>

      {!collapsed && (
        <div className="preset-grid" role="region" aria-label="供应商预设列表">
          {categories.map((cat) => (
            <div className="preset-category" key={cat}>
              <h3 className="preset-category-label">
                {categoryLabels[cat] || cat}
              </h3>
              <div className="preset-category-items">
                {PRESETS.filter((p) => p.category === cat).map((preset) => (
                  <button
                    className="preset-item"
                    key={preset.id}
                    onClick={() => onSelect(createPresetPatch(preset))}
                    title={`${preset.websiteUrl ?? ""}\n${preset.baseUrl}`}
                    type="button"
                  >
                    <span className="preset-item-name">{preset.name}</span>
                    <span className="preset-item-protocol">
                      {preset.protocol === "chatCompletions"
                        ? "Chat"
                        : "Responses"}
                    </span>
                    <span className="preset-item-model">{preset.model}</span>
                  </button>
                ))}
              </div>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}