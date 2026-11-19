/** 自定义请求头的编辑行（#1685 / #3075）：直连模式「高级兼容选项」和网关供应商池的编辑弹窗共用。
 * 一行一个头（名称、值、删除），下面是「添加请求头」。敏感头已保存时值不回显，输入框里提示打码后的样子，留空表示不改。 */
import { Plus, Trash2 } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { t, tf } from "./i18n";
import type { ProviderHeaderRow } from "./relay-headers";

export function RelayHeaderRows<Row extends ProviderHeaderRow>({
  rows,
  onChange,
  disabled = false,
}: {
  rows: Row[];
  onChange: (rows: Row[]) => void;
  disabled?: boolean;
}) {
  const patch = (index: number, change: Partial<ProviderHeaderRow>) => {
    const next = rows.slice();
    next[index] = { ...next[index], ...change };
    onChange(next);
  };
  return (
    <>
      {rows.map((row, index) => (
        <div className="relay-custom-header-row" key={`custom-header-${index}`}>
          <Input
            aria-label={t("请求头名称")}
            disabled={disabled}
            onChange={(event) => patch(index, { key: event.currentTarget.value })}
            placeholder="X-Tenant"
            spellCheck={false}
            value={row.key}
          />
          <Input
            aria-label={t("请求头值")}
            autoComplete="off"
            disabled={disabled}
            onChange={(event) => patch(index, { value: event.currentTarget.value })}
            placeholder={row.masked ? tf("已保存 {0}，留空不改", [row.masked]) : t("请求头值")}
            spellCheck={false}
            value={row.value}
          />
          <Button
            aria-label={t("删除这一项")}
            disabled={disabled}
            onClick={() => onChange(rows.filter((_, i) => i !== index))}
            size="sm"
            type="button"
            variant="secondary"
          >
            <Trash2 className="h-4 w-4" />
          </Button>
        </div>
      ))}
      <div className="relay-custom-headers-actions">
        <Button
          disabled={disabled}
          onClick={() => onChange([...rows, { key: "", value: "" } as Row])}
          size="sm"
          type="button"
          variant="secondary"
        >
          <Plus className="h-4 w-4" />
          {t("添加请求头")}
        </Button>
      </div>
    </>
  );
}
