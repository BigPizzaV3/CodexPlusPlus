import { useCallback, useRef, type MouseEvent } from "react";
import { appConfirm } from "./app-dialog";
import { t } from "./i18n";
import { useEscapeLayer } from "./use-escape-layer";
import { useModalFocus } from "./use-modal-focus";

export type ModalShellOptions = {
  onClose: () => void;
  /** 表单改过：Esc、点遮罩、点关闭都先确认再丢弃。 */
  dirty?: boolean;
  /** 标题元素的 id，作为对话框的无障碍名称。 */
  labelledBy?: string;
  /** 点遮罩是否关闭；默认是。 */
  dismissOnBackdrop?: boolean;
  active?: boolean;
};

/**
 * 弹窗的公共行为，不改调用方的标记和样式：
 * `backdropProps` 给遮罩（点遮罩关闭），`dialogProps` 给卡片（role=dialog、aria-modal、焦点圈在里面），
 * Esc 走全局 Esc 栈；有未保存修改时关闭前先确认。
 */
export function useModalShell<T extends HTMLElement = HTMLElement>({ onClose, dirty = false, labelledBy, dismissOnBackdrop = true, active = true }: ModalShellOptions) {
  const ref = useRef<T>(null);
  const closeRef = useRef(onClose);
  closeRef.current = onClose;
  const dirtyRef = useRef(dirty);
  dirtyRef.current = dirty;
  useModalFocus(ref, active);

  const requestClose = useCallback(async () => {
    if (dirtyRef.current) {
      const discard = await appConfirm(t("有未保存的修改，关闭后会丢失。"), { title: t("放弃修改？"), confirmText: t("放弃修改"), danger: true });
      if (!discard) return;
    }
    closeRef.current();
  }, []);
  useEscapeLayer(active, () => void requestClose());

  return {
    requestClose,
    backdropProps: {
      onMouseDown: (event: MouseEvent<HTMLElement>) => {
        if (dismissOnBackdrop && event.target === event.currentTarget) void requestClose();
      },
    },
    dialogProps: {
      ref,
      role: "dialog" as const,
      "aria-modal": true as const,
      "aria-labelledby": labelledBy,
      tabIndex: -1,
    },
  };
}
