/** 供应商池详情里的「Key」一行（#2782）：列出已保存的 Key（只显示头尾），每个可单独替换、删除，也能再加一个。
 *
 * Key 原文从不发到界面：后端只给标识和打码后的样子，替换 / 删除 / 测试按标识操作；新填的 Key 只在提交时发给后端。
 */
import { useState, type FormEvent } from "react";
import { Pencil, Plus, Trash2, Zap } from "lucide-react";
import { appConfirm } from "./app-dialog";
import { t, tf } from "./i18n";
import type { GatewayPoolEntry } from "./gateway-model";
import type { Run } from "./gateway-ui";

/** 正在填写的 Key：替换某个已保存的 Key，或新加一个。 */
type Editing = { mode: "replace"; keyId: string; masked: string } | { mode: "add" };

export function ProviderKeyList({ entry, busy, run, onTest }: { entry: GatewayPoolEntry; busy: string | null; run: Run; onTest: (keyId: string) => void }) {
  const keys = entry.keys ?? [];
  const [editing, setEditing] = useState<Editing | null>(null);
  const [value, setValue] = useState("");
  const open = (next: Editing) => {
    setValue("");
    setEditing(next);
  };
  const close = () => {
    setValue("");
    setEditing(null);
  };
  const submit = async (event: FormEvent) => {
    event.preventDefault();
    const key = value.trim();
    if (!editing || !key) return;
    const ok = editing.mode === "add"
      ? await run(`key-add:${entry.id}`, "add_gateway_provider_key", { id: entry.id, key }, t("添加 Key"))
      : await run(`key-replace:${entry.id}`, "replace_gateway_provider_key", { id: entry.id, keyId: editing.keyId, key }, t("替换 Key"));
    if (ok) close();
  };
  const remove = async (keyId: string, masked: string) => {
    const ok = await appConfirm(tf("删除 Key「{0}」？删除后它不再参与轮换，用它的会话会换用别的 Key。", [masked]), { title: t("删除 Key"), confirmText: t("删除"), danger: true });
    if (ok) await run(`key-delete:${entry.id}`, "delete_gateway_provider_key", { id: entry.id, keyId }, t("删除 Key"));
  };
  const last = keys.length < 2;
  return (
    <div className="gw-key-list">
      {keys.map((key) => (
        <span className="gw-key-item" data-editing={editing?.mode === "replace" && editing.keyId === key.id} key={key.id}>
          <code className="gw-chip">{key.masked}</code>
          {/* 只用这个 Key 发一条测试请求（按标识选，Key 原文不经过界面）。 */}
          <button
            aria-label={tf("测试 Key {0}", [key.masked])}
            className="gw-icon-button gw-inline-icon"
            disabled={busy !== null}
            onClick={() => onTest(key.id)}
            title={t("只用这个 Key 发一条测试请求")}
            type="button"
          >
            <Zap className="h-3.5 w-3.5" />
          </button>
          <button
            aria-label={tf("替换 Key {0}", [key.masked])}
            className="gw-icon-button gw-inline-icon"
            disabled={busy !== null}
            onClick={() => open({ mode: "replace", keyId: key.id, masked: key.masked })}
            title={t("替换这个 Key")}
            type="button"
          >
            <Pencil className="h-3.5 w-3.5" />
          </button>
          <button
            aria-label={tf("删除 Key {0}", [key.masked])}
            className="gw-icon-button gw-inline-icon gw-inline-danger"
            disabled={busy !== null || last}
            onClick={() => void remove(key.id, key.masked)}
            title={last ? t("至少要保留一个 Key，要换掉它请用替换") : t("删除这个 Key")}
            type="button"
          >
            <Trash2 className="h-3.5 w-3.5" />
          </button>
        </span>
      ))}
      {editing ? (
        <form className="gw-key-editor" onSubmit={(event) => void submit(event)}>
          <input
            aria-label={editing.mode === "add" ? t("新的 API Key") : tf("替换 Key {0}", [editing.masked])}
            autoComplete="off"
            autoFocus
            className="gw-inline-input gw-key-input"
            onChange={(event) => setValue(event.currentTarget.value)}
            onKeyDown={(event) => {
              if (event.key === "Escape") {
                event.stopPropagation();
                close();
              }
            }}
            placeholder={editing.mode === "add" ? "sk-..." : tf("替换 {0} 的新 Key", [editing.masked])}
            spellCheck={false}
            type="password"
            value={value}
          />
          <button className="gw-text-button strong" disabled={busy !== null || !value.trim()} type="submit">{t("保存")}</button>
          <button className="gw-text-button" onClick={close} type="button">{t("取消")}</button>
        </form>
      ) : (
        <button className="gw-text-button" disabled={busy !== null} onClick={() => open({ mode: "add" })} type="button">
          <Plus className="h-3.5 w-3.5" />
          {t("添加 Key")}
        </button>
      )}
    </div>
  );
}
