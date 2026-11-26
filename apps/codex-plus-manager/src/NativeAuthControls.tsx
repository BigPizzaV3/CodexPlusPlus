import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { Check, Loader2, RefreshCw, RotateCw } from "lucide-react";
import { t } from "./i18n";
import { loginIdentityArgs, nativeAuthSelection, savedAuthLabel, type NativeAuthResult, type NativeAuthState } from "./native-auth-model";
import { AppSelect } from "./AppSelect";
import { CxCard, CxRow } from "./cx-layout";
import { AgentIcon } from "./gateway-ui";
import { LocalAccountProfile } from "./ChatgptCompatSection";
import type { BackendSettings } from "./app-types";
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
    // 别处改了登录身份（Codex 里的供应商页、另一个窗口）：后端发设置变更事件，这里跟着重读。
    let disposed = false;
    let stop: (() => void) | undefined;
    void listen("codex-plus-settings-changed", () => { if (!disposed) void reload(); })
      .then((unlisten) => { if (disposed) unlisten(); else stop = unlisten; })
      .catch(() => {});
    return () => {
      disposed = true;
      stop?.();
      sequence.current += 1;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);
  return { state, setState, loading, error, setError, reload };
}

/**
 * 「Codex 登录身份」：三选一，ChatGPT 账号 / 本地账号（用 API Key 解锁 ChatGPT 专属入口）/ 不登录。
 * 后端 `set_codex_login_identity` 一起改原生登录选择和解锁开关，两边写 auth.json 不互相覆盖。
 */
export function NativeAuthControl({ onRestart, form, onSave }: {
  onRestart?: () => Promise<boolean>;
  /** 有这两项才显示本地账号的「编辑资料」。 */
  form?: BackendSettings;
  onSave?: (next: BackendSettings) => void;
}) {
  const { state, setState, loading, error, setError, reload } = useSavedAuthAccounts();
  const [chosen, setChosen] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const [restarting, setRestarting] = useState(false);
  const current = state ? nativeAuthSelection(state) : "";
  const value = chosen ?? current;
  // 下拉里选了别的账号但还没写入。
  const pending = chosen !== null && chosen !== current;
  const busy = loading || saving || restarting;
  const save = async () => {
    if (!state || !value || saving || loading) return;
    setSaving(true);
    setError("");
    try {
      const result = await invoke<NativeAuthResult>("set_codex_login_identity", loginIdentityArgs(value));
      if (result.status !== "ok") throw new Error(result.message);
      setState(result);
      setChosen(null);
    } catch (failure) {
      await reload();
      setError(String(failure));
    } finally {
      setSaving(false);
    }
  };
  const restart = async () => {
    if (!onRestart || busy) return;
    setRestarting(true);
    setError("");
    try {
      if (await onRestart()) {
        setChosen(null);
        await reload();
      }
    } catch (failure) {
      setError(String(failure));
    } finally {
      setRestarting(false);
    }
  };
  // 提示一次只显示一条，放进卡片里，不再在卡片外面散成几行：未应用的选择 > 错误 > 配置不一致 > 不能重启的原因 > 本地账号要从 Codex++ 启动 > 需要重启。
  const notice: { text: string; tone: "error" | "muted"; role?: "alert" | "status" } | null = pending
    ? { text: t("选择还没生效，点「应用」后才会写入 Codex 的登录凭证。"), tone: "muted", role: "status" }
    : error
      ? { text: error, tone: "error", role: "alert" }
      : state?.drift
        ? { text: state.drift, tone: "error", role: "status" }
        : state?.restartBlockedReason
          ? { text: t(state.restartBlockedReason), tone: "muted", role: "status" }
          : state?.localLoginLaunchHint && value === "local"
            ? { text: t(state.localLoginLaunchHint), tone: "muted", role: "status" }
          : state?.managed
            ? { text: t("更改登录选择后，请关闭所有 Codex 实例并重新启动。"), tone: "muted", role: "status" }
            : null;
  return (
    <CxCard className="native-auth-control">
      <CxRow
        className="native-auth-current"
        detail={state ? `${t("已写入凭证：")}${state.actualLabel || t("未登录")}` : undefined}
        icon={<AgentIcon agent="codex" />}
        // 「Codex 登录身份」：Codex 自己用哪个身份登录；和各供应商绑定的「供应商登录账号」区分开。
        title={t("Codex 登录身份")}
      >
        <AppSelect
          ariaLabel={t("Codex 当前登录")}
          disabled={!state || busy}
          id="native-auth-account"
          onChange={setChosen}
          size="sm"
          options={[
            ...(!current ? [{ value: "", label: state?.actualLabel || t("尚未指定") }] : []),
            {
              value: "local",
              label: t("本地账号"),
              // 官方登录供应商下模型请求要用真实登录，换成本地账号就连不上：置灰并说明原因。
              hint: state?.localLoginBlockedReason ? t(state.localLoginBlockedReason) : t("用 API Key 解锁代码审查、图像、资料库等入口，模型仍走所选供应商"),
              disabled: Boolean(state?.localLoginBlockedReason) && current !== "local",
            },
            { value: "none", label: t("不登录"), hint: t("保留已保存账号") },
            ...(state?.accounts.map((account) => ({ value: `account:${account.id}`, label: savedAuthLabel(account) })) ?? []),
            ...(current.startsWith("account:") && !state?.accounts.some((account) => `account:${account.id}` === current) ? [{ value: current, label: t("已选账号不可用") }] : []),
          ]}
          value={value}
        />
        {/* 下拉里换了账号还没应用：给一个带字的「应用」按钮，不再只是一个容易漏看的 ✓ 图标。 */}
        {pending || saving ? (
          <button aria-label={saving ? t("保存中") : t("应用当前登录选择")} className="cx-primary native-auth-apply" disabled={!state || !value || busy} onClick={() => void save()} title={t("应用当前登录选择")} type="button">
            {saving ? <Loader2 className="animate-spin" /> : <Check />}
            {saving ? t("保存中") : t("应用")}
          </button>
        ) : null}
        {state?.managed && onRestart ? (
          // 和右边的「刷新」图标长得太像：重启写成带字的按钮。
          <button aria-label={t("重启 Codex 应用登录选择")} className="cx-secondary native-auth-restart" disabled={busy || chosen !== null || !!state.drift || !!state.restartBlockedReason} onClick={() => void restart()} title={t("重启 Codex 应用登录选择")} type="button">
            <RotateCw className={restarting ? "animate-spin" : ""} />
            {t("重启 Codex")}
          </button>
        ) : null}
        <button aria-label={t("刷新登录账号")} className="native-auth-action" disabled={busy} onClick={() => { setChosen(null); void reload(); }} title={t("刷新登录账号")} type="button">
          <RefreshCw className={loading ? "animate-spin" : ""} />
        </button>
      </CxRow>
      {value === "local" && form && onSave ? <LocalAccountProfile form={form} onSave={onSave} /> : null}
      {notice ? <p className={notice.tone === "error" ? "native-auth-error native-auth-notice" : "native-auth-pending native-auth-notice"} role={notice.role}>{notice.text}</p> : null}
    </CxCard>
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
