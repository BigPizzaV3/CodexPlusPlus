// 「连接与供应商」页的「用 API Key 解锁 ChatGPT 专属入口」：开关 + 本地账号资料。
// 只在 API Key 供应商 / 本地服务下生效（后端遇到官方登录会自己拒绝），所以放在供应商页。
import { useEffect, useState } from "react";
import { Input } from "@/components/ui/input";
import { t } from "@/i18n";
import type { BackendSettings } from "./app-types";
import { ToggleVisual } from "./app-shared";
import { CxRow } from "./cx-layout";

type ProfileKey = "chatgptCompatName" | "chatgptCompatUsername" | "chatgptCompatEmail";

/// 文本框先改本地草稿，失焦或回车才保存：每敲一个字都存一次设置太重。
function ProfileInput({ field, form, label, placeholder, type, onSave }: {
  field: ProfileKey;
  form: BackendSettings;
  label: string;
  placeholder: string;
  type?: string;
  onSave: (next: BackendSettings) => void;
}) {
  const saved = form[field] ?? "";
  const [draft, setDraft] = useState(saved);
  // 设置从别处改动（或保存后回读）时跟上。
  useEffect(() => setDraft(saved), [saved]);
  const commit = () => {
    const value = draft.trim();
    if (value !== saved) onSave({ ...form, [field]: value });
  };
  return (
    <Input
      aria-label={label}
      onBlur={commit}
      onChange={(event) => setDraft(event.currentTarget.value)}
      onKeyDown={(event) => { if (event.key === "Enter") event.currentTarget.blur(); }}
      placeholder={placeholder}
      spellCheck={false}
      title={label}
      type={type}
      value={draft}
    />
  );
}

/** 「用 API Key 解锁 ChatGPT 专属入口」的两行，放在「连接与供应商」页底部「更多设置」折叠区里。 */
export function ChatgptCompatRows({ form, onSave }: {
  form: BackendSettings;
  onSave: (next: BackendSettings) => void;
}) {
  const enabled = Boolean(form.chatgptCompatEnabled);
  return (
    <>
        <CxRow
          className="cx-preserve-row"
          detail={t("代码审查、图像、资料库等入口原本只在 ChatGPT 登录下可见；模型请求仍走你选的供应商。重新启动 Codex 后生效。")}
          tooltip={t("开启后 Codex++ 在本机 8000 端口起一个本地 ChatGPT 兼容服务，并改写 ~/.codex 的配置和登录（先备份，关闭时还原）。只在用 API Key 供应商或本地服务时生效，官方登录时不改动。")}
          title={t("用 API Key 解锁 ChatGPT 专属入口")}
        >
          <label className="switch-row compact relay-master-switch relay-master-switch-compact">
            <input
              aria-label={t("用 API Key 解锁 ChatGPT 专属入口")}
              checked={enabled}
              onChange={(event) => onSave({ ...form, chatgptCompatEnabled: event.currentTarget.checked })}
              type="checkbox"
            />
            <span>{t("解锁")}</span>
            <ToggleVisual />
          </label>
        </CxRow>
        {enabled ? (
          <CxRow
            className="cx-preserve-row"
            detail={t("Codex 里显示的名字、用户名和邮箱，只在本机使用。留空用默认值。重新启动 Codex 后生效。")}
            title={t("本地账号资料")}
          >
            <div className="chatgpt-compat-profile">
              <ProfileInput field="chatgptCompatName" form={form} label={t("名字")} onSave={onSave} placeholder="Local" />
              <ProfileInput field="chatgptCompatUsername" form={form} label={t("用户名")} onSave={onSave} placeholder="local" />
              <ProfileInput field="chatgptCompatEmail" form={form} label={t("邮箱")} onSave={onSave} placeholder="local@codexplus.invalid" type="email" />
            </div>
          </CxRow>
        ) : null}
    </>
  );
}
