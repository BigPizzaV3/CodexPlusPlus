// 设置表单的字段描述（拓展清单 config 与 dsh 插件 Config 共用），以及 Schemastery JSON → 字段的转换。

export type LocalizedText = string | Record<string, string>;
export type ExtensionConfigField = {
  key: string;
  type: "string" | "text" | "number" | "boolean" | "select" | "array" | "object";
  title: LocalizedText;
  description: LocalizedText;
  default: unknown;
  options: Array<{ value: unknown; label: string }>;
  secret: boolean;
  min?: number;
  max?: number;
  items?: ExtensionConfigField;
  properties?: ExtensionConfigField[];
  /** string 的输入方式：file 表示本机文件路径，表单旁边给一个「选择…」按钮。 */
  format?: "file";
  /** format 为 file 时选择器只列这些扩展名（不带点）；缺省不限。 */
  extensions?: string[];
};

/** Schemastery 的 toJSON（dsh 插件的 Config）。 */
export type SchemasteryJson = {
  type?: string;
  dict?: Record<string, SchemasteryJson>;
  inner?: SchemasteryJson;
  list?: SchemasteryJson[];
  value?: unknown;
  meta?: { default?: unknown; required?: boolean; description?: LocalizedText; role?: string; min?: number; max?: number; hidden?: boolean };
};

/// dsh 插件的 Config（Schemastery JSON）→ 设置表单字段。顶层必须是 object；表单表达不了的字段（dict、any、
/// 混合 union……）略过，留给「高级：JSON」。没有任何可表达的字段时返回空数组。
export function schemasteryFields(schema: SchemasteryJson | null | undefined): ExtensionConfigField[] {
  if (!schema || schema.type !== "object" || !schema.dict) return [];
  return Object.entries(schema.dict).flatMap(([key, child]) => {
    const field = schemasteryField(key, child);
    return field ? [field] : [];
  });
}

function schemasteryField(key: string, schema: SchemasteryJson | undefined): ExtensionConfigField | null {
  if (!schema || schema.meta?.hidden) return null;
  const meta = schema.meta ?? {};
  const base = { key, title: key, description: meta.description ?? "", default: meta.default, options: [], secret: false };
  switch (schema.type) {
    case "string":
      return { ...base, type: meta.role === "textarea" ? "text" : "string", secret: meta.role === "secret" };
    case "number":
    case "natural":
    case "percent":
      return { ...base, type: "number", min: meta.min ?? (schema.type === "natural" ? 0 : undefined), max: meta.max ?? (schema.type === "percent" ? 1 : undefined) };
    case "boolean":
      return { ...base, type: "boolean" };
    case "union": {
      const list = schema.list ?? [];
      if (!list.length || !list.every((option) => option.type === "const")) return null;
      return {
        ...base,
        type: "select",
        options: list.map((option) => ({ value: option.value, label: typeof option.meta?.description === "string" ? option.meta.description : String(option.value) })),
      };
    }
    case "array": {
      const items = schemasteryField("item", schema.inner);
      return items && items.type !== "object" ? { ...base, type: "array", items, default: meta.default ?? [] } : null;
    }
    case "object": {
      const properties = schemasteryFields(schema);
      return properties.length ? { ...base, type: "object", properties } : null;
    }
    default:
      return null;
  }
}
