/**
 * 应用内的确认 / 提示 / 输入框，替代系统原生的对话框（window.confirm / alert / prompt 与
 * 插件的 confirm）。任何页面直接 `await appConfirm(...)`；`<AppDialogHost />` 在主窗口里挂一次。
 */
import { useEffect, useRef, useState, useSyncExternalStore } from "react";
import { Button } from "@/components/ui/button";
import { t } from "./i18n";
import { useEscapeLayer } from "./use-escape-layer";

type DialogKind = "confirm" | "alert" | "prompt";

type DialogRequest = {
  id: number;
  kind: DialogKind;
  title: string;
  message: string;
  confirmText: string;
  cancelText: string;
  /** 危险操作（删除、清空等）：确认按钮用醒目的颜色。 */
  danger: boolean;
  defaultValue: string;
  placeholder: string;
  resolve: (value: boolean | string | null) => void;
};

export type ConfirmOptions = {
  title?: string;
  confirmText?: string;
  cancelText?: string;
  danger?: boolean;
};

let queue: DialogRequest[] = [];
let nextId = 1;
const listeners = new Set<() => void>();

function emit() {
  for (const listener of listeners) listener();
}

function enqueue(request: Omit<DialogRequest, "id">) {
  queue = [...queue, { ...request, id: nextId++ }];
  emit();
}

function finish(id: number) {
  queue = queue.filter((request) => request.id !== id);
  emit();
}

/** 确认框：确认返回 true，取消 / Esc / 点遮罩返回 false。 */
export function appConfirm(message: string, options: ConfirmOptions = {}): Promise<boolean> {
  return new Promise((resolve) =>
    enqueue({
      kind: "confirm",
      title: options.title ?? t("请确认"),
      message,
      confirmText: options.confirmText ?? t("确定"),
      cancelText: options.cancelText ?? t("取消"),
      danger: options.danger ?? false,
      defaultValue: "",
      placeholder: "",
      resolve: (value) => resolve(value === true),
    }),
  );
}

/** 提示框：只有一个「知道了」。 */
export function appAlert(message: string, options: { title?: string } = {}): Promise<void> {
  return new Promise((resolve) =>
    enqueue({
      kind: "alert",
      title: options.title ?? t("提示"),
      message,
      confirmText: t("知道了"),
      cancelText: "",
      danger: false,
      defaultValue: "",
      placeholder: "",
      resolve: () => resolve(),
    }),
  );
}

/** 输入框：确认返回输入的文字，取消返回 null。 */
export function appPrompt(
  title: string,
  options: { defaultValue?: string; placeholder?: string; message?: string; confirmText?: string } = {},
): Promise<string | null> {
  return new Promise((resolve) =>
    enqueue({
      kind: "prompt",
      title,
      message: options.message ?? "",
      confirmText: options.confirmText ?? t("确定"),
      cancelText: t("取消"),
      danger: false,
      defaultValue: options.defaultValue ?? "",
      placeholder: options.placeholder ?? "",
      resolve: (value) => resolve(typeof value === "string" ? value : null),
    }),
  );
}

function subscribe(listener: () => void) {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

/** 挂在主窗口里，依次显示排队的对话框。 */
export function AppDialogHost() {
  const pending = useSyncExternalStore(subscribe, () => queue, () => queue);
  const current = pending[0];
  if (!current) return null;
  return <AppDialog key={current.id} request={current} />;
}

function AppDialog({ request }: { request: DialogRequest }) {
  const [value, setValue] = useState(request.defaultValue);
  const confirmRef = useRef<HTMLButtonElement>(null);
  const inputRef = useRef<HTMLInputElement>(null);
  const close = (result: boolean | string | null) => {
    request.resolve(result);
    finish(request.id);
  };
  const cancel = () => close(request.kind === "prompt" ? null : false);
  const confirm = () => close(request.kind === "prompt" ? value : true);

  // Esc 走全局的 Esc 栈：确认框总是后打开的，只关它自己，不连带关掉下面的弹窗。
  useEscapeLayer(true, cancel);

  useEffect(() => {
    if (request.kind === "prompt") {
      inputRef.current?.focus();
      inputRef.current?.select();
    } else {
      confirmRef.current?.focus();
    }
  }, [request.kind]);

  return (
    <div
      aria-modal="true"
      className="modal-backdrop app-dialog-backdrop"
      onMouseDown={(event) => {
        if (event.target === event.currentTarget) cancel();
      }}
      role="dialog"
    >
      <div className="modal-card confirm-modal app-dialog" data-danger={request.danger || undefined}>
        <div className="modal-head">
          <div>
            <h2>{request.title}</h2>
          </div>
          <button aria-label={t("关闭")} className="toast-close" onClick={cancel} type="button">×</button>
        </div>
        <div className="confirm-modal-body">
          {request.message ? <p className="modal-message app-dialog-message">{request.message}</p> : null}
          {request.kind === "prompt" ? (
            <input
              className="app-dialog-input"
              onChange={(event) => setValue(event.currentTarget.value)}
              onKeyDown={(event) => {
                if (event.key === "Enter") {
                  event.preventDefault();
                  confirm();
                }
              }}
              placeholder={request.placeholder}
              ref={inputRef}
              value={value}
            />
          ) : null}
        </div>
        <div className="app-dialog-actions">
          {request.kind === "alert" ? null : (
            <Button onClick={cancel} variant="secondary">{request.cancelText}</Button>
          )}
          <Button className={request.danger ? "app-dialog-danger" : undefined} onClick={confirm} ref={confirmRef}>
            {request.confirmText}
          </Button>
        </div>
      </div>
    </div>
  );
}
