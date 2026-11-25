// 「本地账号」（用 API Key 解锁 ChatGPT 专属入口）的资料编辑。
// 本地账号本身是「Codex 登录身份」下拉的一项（NativeAuthControls.tsx），这里只有名字、用户名、邮箱。
import { useEffect, useState } from "react";
import { Input } from "@/components/ui/input";
import { t } from "@/i18n";
import type { BackendSettings } from "./app-types";

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

/** 「Codex 登录身份」选「本地账号」时的资料：平时折着，点「编辑资料」才展开。 */
export function LocalAccountProfile({ form, onSave }: {
  form: BackendSettings;
  onSave: (next: BackendSettings) => void;
}) {
  return (
    <details className="cx-set-disclosure local-account-profile">
      <summary>
        <span>{t("编辑资料")}</span>
        <span className="cx-set-summary-hint">{t("Codex 里显示的名字、用户名和邮箱，只在本机使用，留空用默认值")}</span>
      </summary>
      <div className="local-account-fields">
        <ProfileInput field="chatgptCompatName" form={form} label={t("名字")} onSave={onSave} placeholder="Local" />
        <ProfileInput field="chatgptCompatUsername" form={form} label={t("用户名")} onSave={onSave} placeholder="local" />
        <ProfileInput field="chatgptCompatEmail" form={form} label={t("邮箱")} onSave={onSave} placeholder="local@codexplus.invalid" type="email" />
      </div>
    </details>
  );
}
