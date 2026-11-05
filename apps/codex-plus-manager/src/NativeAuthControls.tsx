import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Check, Loader2, RefreshCw } from "lucide-react";
import { t } from "./i18n";
import { nativeAuthSelection, savedAuthLabel, type NativeAuthResult, type NativeAuthState } from "./native-auth-model";
import { AppSelect } from "./AppSelect";
import { CxRow } from "./cx-layout";
import { AgentIcon } from "./gateway-ui";
import "./NativeAuthControls.css";

function useSavedAuthAccounts() {
  const [state, setState] = useState<NativeAuthState | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState("");
  const sequence = useRef(0);
  const reload = async () => {
    const current = ++sequence.current;
    setLoading(true);
    setError("");
    try {
      const result = await invoke<NativeAuthResult>("get_native_auth_state");
      if (current !== sequence.current) return;
      if (result.status !== "ok") throw new Error(result.message);
      setState(result);
    } catch (failure) {
      if (current === sequence.current) setError(String(failure));
    } finally {
      if (current === sequence.current) setLoading(false);
    }
  };
  useEffect(() => {
    void reload();
    return () => { sequence.current += 1; };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);
  return { state, setState, loading, error, setError, reload };
}

export function NativeAuthControl() {
  const { state, setState, loading, error, setError, reload } = useSavedAuthAccounts();
  const [chosen, setChosen] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const current = state ? nativeAuthSelection(state) : "";
  const value = chosen ?? current;
  const save = async () => {
    if (!state || !value || saving || loading) return;
    setSaving(true);
    setError("");
    try {
      const result = await invoke<NativeAuthResult>("set_native_auth_account", {
        accountRef: value === "none" ? null : value.slice("account:".length),
      });
      if (result.status !== "ok") throw new Error(result.message);
      setState(result);
      setChosen(null);
    } catch (failure) {
      setError(String(failure));
    } finally {
      setSaving(false);
    }
  };
  return (
    <div className="native-auth-control">
      <CxRow className="native-auth-current" icon={<AgentIcon agent="codex" />} title={t("Codex 登录")}>
        <AppSelect
          ariaLabel={t("Codex 当前登录")}
          disabled={!state || loading || saving}
          id="native-auth-account"
          onChange={setChosen}
          size="sm"
          options={[
            ...(!current ? [{ value: "", label: state?.actualLabel || t("尚未指定") }] : []),
            { value: "none", label: t("不登录"), hint: t("保留已保存账号") },
            ...(state?.accounts.map((account) => ({ value: `account:${account.id}`, label: savedAuthLabel(account) })) ?? []),
            ...(current.startsWith("account:") && !state?.accounts.some((account) => `account:${account.id}` === current) ? [{ value: current, label: t("已选账号不可用") }] : []),
          ]}
          value={value}
        />
        <button aria-label={saving ? t("保存中") : t("应用当前登录选择")} className="native-auth-action" disabled={!state || !value || loading || saving || (value === current && !state.drift)} onClick={() => void save()} title={t("应用当前登录选择")} type="button">
          {saving ? <Loader2 className="animate-spin" /> : <Check />}
        </button>
        <button aria-label={t("刷新登录账号")} className="native-auth-action" disabled={loading || saving} onClick={() => { setChosen(null); void reload(); }} title={t("刷新登录账号")} type="button">
          <RefreshCw className={loading ? "animate-spin" : ""} />
        </button>
      </CxRow>
      {error ? <p className="native-auth-error" role="alert">{error}</p> : state?.drift ? <p className="native-auth-error" role="status">{state.drift} · {t("实际登录：")}{state.actualLabel || t("未登录")}</p> : null}
    </div>
  );
}

/** 供应商绑定只保存引用；当前原生登录由上方独立控件选择。 */
export function SavedAuthAccountField({ value, onChange }: { value: string; onChange: (value: string) => void }) {
  const { state, loading, error, reload } = useSavedAuthAccounts();
  return (
    <div className="native-auth-control">
      <div className="native-auth-row">
        <label>
          <span>{t("供应商登录账号")}</span>
          <AppSelect
            ariaLabel={t("供应商登录账号")}
            disabled={!state || loading}
            onChange={onChange}
            options={[
              { value: "", label: t("未绑定账号") },
              ...(state?.accounts.map((account) => ({ value: account.id, label: savedAuthLabel(account) })) ?? []),
              ...(value && !state?.accounts.some((account) => account.id === value) ? [{ value, label: t("已绑定账号不可用") }] : []),
            ]}
            value={value}
          />
        </label>
        <button aria-label={t("刷新登录账号")} className="gw-icon-button" disabled={loading} onClick={() => void reload()} title={t("刷新登录账号")} type="button"><RefreshCw className={loading ? "h-3.5 w-3.5 animate-spin" : "h-3.5 w-3.5"} /></button>
      </div>
      {error ? <p className="native-auth-error" role="alert">{error}</p> : null}
    </div>
  );
}
