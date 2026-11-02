import { useState } from "react";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { getLanguage, t, tf } from "@/i18n";
import { AppSelect } from "./AppSelect";
import type { ExtensionConfigField, LocalizedText } from "./extension-config-schema";

export type { ExtensionConfigField, LocalizedText, SchemasteryJson } from "./extension-config-schema";
export { schemasteryFields } from "./extension-config-schema";

// 拓展清单 config 与 dsh 插件 Config 共用的设置表单（从 App.tsx 拆出，「dsh 插件」分栏也用它）。

function ToggleVisual() {
  return (
    <span aria-hidden="true" className="toggle-switch-visual">
      <span className="toggle-switch-thumb" />
    </span>
  );
}

/// 按拓展清单的 config schema 生成的设置表单。值类型严格按 schema：数字框转成数字、开关是布尔、下拉取选项原值。
/// 清单里的多语言文本：先找当前界面语言，再找 en，最后取第一个。
export function localizedText(text: LocalizedText | undefined): string {
  if (!text) return "";
  if (typeof text === "string") return text;
  for (const lang of [getLanguage(), "en"]) {
    const hit = Object.keys(text).find((key) => key.toLowerCase() === lang || key.toLowerCase().startsWith(`${lang}-`));
    if (hit) return text[hit];
  }
  return Object.values(text)[0] ?? "";
}

export function ExtensionConfigForm({ fields, values, onSave, onCancel, saveLabel }: {
  fields: ExtensionConfigField[];
  /** 保存按钮的文字，默认「保存并热重载」。 */
  saveLabel?: string;
  values: Record<string, unknown>;
  onSave: (values: Record<string, unknown>) => Promise<void>;
  onCancel: () => void;
}) {
  const [draft, setDraft] = useState<Record<string, unknown>>(() => ({ ...values }));
  const [saving, setSaving] = useState(false);
  return (
    <form
      className="extension-config-form"
      onSubmit={(event) => {
        event.preventDefault();
        setSaving(true);
        void onSave(draft).finally(() => setSaving(false));
      }}
    >
      {fields.map((field) => (
        <ExtensionConfigFieldRow
          field={field}
          idPrefix="extension-config"
          key={field.key}
          onChange={(value) => setDraft((current) => ({ ...current, [field.key]: value }))}
          value={draft[field.key] ?? field.default}
        />
      ))}
      <div className="extension-config-actions">
        <Button disabled={saving} onClick={onCancel} size="sm" type="button" variant="ghost">{t("取消")}</Button>
        <Button disabled={saving} size="sm" type="submit">{saving ? t("保存中…") : (saveLabel ?? t("保存并热重载"))}</Button>
      </div>
    </form>
  );
}

/// 一个配置字段：标题 + 编辑器 + 说明。对象字段递归展开成一组子字段。
function ExtensionConfigFieldRow({ field, value, onChange, idPrefix }: {
  field: ExtensionConfigField;
  value: unknown;
  onChange: (value: unknown) => void;
  idPrefix: string;
}) {
  const id = `${idPrefix}-${field.key}`;
  const title = localizedText(field.title) || field.key;
  const description = localizedText(field.description);
  if (field.type === "object") {
    const object = value && typeof value === "object" && !Array.isArray(value) ? (value as Record<string, unknown>) : {};
    return (
      <fieldset className="extension-config-group">
        <legend>{title}</legend>
        {description ? <small>{description}</small> : null}
        {(field.properties ?? []).map((child) => (
          <ExtensionConfigFieldRow
            field={child}
            idPrefix={id}
            key={child.key}
            onChange={(next) => onChange({ ...object, [child.key]: next })}
            value={object[child.key] ?? child.default}
          />
        ))}
      </fieldset>
    );
  }
  return (
    <div className="extension-config-field">
      <label htmlFor={id}>{title}</label>
      <ExtensionConfigEditor field={field} id={id} onChange={onChange} value={value} />
      {description ? <small>{description}</small> : null}
    </div>
  );
}

/// 本机文件路径（清单里 `format: "file"`）：可以直接填，也可以点「选择…」用系统对话框选。
function ExtensionConfigFileInput({ field, value, onChange, id, title }: {
  field: ExtensionConfigField;
  value: unknown;
  onChange: (value: unknown) => void;
  id: string;
  title: string;
}) {
  const [error, setError] = useState("");
  const choose = async () => {
    setError("");
    try {
      const extensions = field.extensions?.length ? field.extensions : undefined;
      const selected = await openDialog({
        directory: false,
        multiple: false,
        title,
        filters: extensions ? [{ name: title, extensions }] : undefined,
      });
      if (typeof selected === "string" && selected.trim()) onChange(selected.trim());
    } catch (reason) {
      setError(tf("打开选择器失败：{0}", [reason instanceof Error ? reason.message : String(reason)]));
    }
  };
  return (
    <div className="extension-config-file">
      <div className="extension-config-file-row">
        <Input id={id} onChange={(event) => onChange(event.currentTarget.value)} value={String(value ?? "")} />
        <Button onClick={() => void choose()} size="sm" type="button" variant="secondary">{t("选择文件")}</Button>
      </div>
      {error ? <small className="extension-config-file-error">{error}</small> : null}
    </div>
  );
}

/// 单个值的编辑器；数组逐项编辑，可增删。
function ExtensionConfigEditor({ field, value, onChange, id }: {
  field: ExtensionConfigField;
  value: unknown;
  onChange: (value: unknown) => void;
  id: string;
}) {
  const title = localizedText(field.title) || field.key;
  if (field.type === "boolean") {
    return (
      <label className="relay-bare-switch">
        <input checked={value === true} id={id} onChange={() => onChange(value !== true)} type="checkbox" />
        <ToggleVisual />
      </label>
    );
  }
  if (field.type === "select") {
    return (
      <AppSelect
        ariaLabel={title}
        id={id}
        onChange={(index) => onChange(field.options[Number(index)]?.value)}
        options={field.options.map((option, index) => ({ value: String(index), label: option.label }))}
        size="sm"
        value={String(Math.max(0, field.options.findIndex((option) => option.value === value)))}
      />
    );
  }
  if (field.type === "text") {
    return (
      <textarea
        className="extension-config-textarea"
        id={id}
        onChange={(event) => onChange(event.currentTarget.value)}
        rows={4}
        value={String(value ?? "")}
      />
    );
  }
  if (field.type === "array" && field.items) {
    const items = Array.isArray(value) ? value : [];
    const item = field.items;
    return (
      <div className="extension-config-array" id={id}>
        {items.map((entry, index) => (
          <div className="extension-config-array-item" key={index}>
            <ExtensionConfigEditor
              field={item}
              id={`${id}-${index}`}
              onChange={(next) => onChange(items.map((current, position) => (position === index ? next : current)))}
              value={entry}
            />
            <Button aria-label={tf("删除第 {0} 项", [index + 1])} onClick={() => onChange(items.filter((_, position) => position !== index))} size="sm" type="button" variant="ghost">
              {t("删除")}
            </Button>
          </div>
        ))}
        <Button onClick={() => onChange([...items, item.default])} size="sm" type="button" variant="secondary">{t("添加一项")}</Button>
      </div>
    );
  }
  if (field.type === "string" && field.format === "file") {
    return <ExtensionConfigFileInput field={field} id={id} onChange={onChange} title={title} value={value} />;
  }
  return (
    <Input
      id={id}
      max={field.type === "number" ? field.max : undefined}
      min={field.type === "number" ? field.min : undefined}
      onChange={(event) => {
        const raw = event.currentTarget.value;
        onChange(field.type === "number" ? (raw.trim() === "" || Number.isNaN(Number(raw)) ? raw : Number(raw)) : raw);
      }}
      type={field.type === "number" ? "number" : field.secret ? "password" : "text"}
      value={String(value ?? "")}
    />
  );
}
