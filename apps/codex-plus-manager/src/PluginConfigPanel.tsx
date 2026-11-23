/**
 * 「Codex 插件」页的「本机配置」：Codex config.toml 里的 `[plugins."<插件>@<市场>"]` 条目，一条一行。
 * 每行一个启用开关，菜单里编辑 / 删除这段配置；末尾一行添加一段新配置。
 * 原来藏在「MCP 与 Skills」三点菜单的「管理插件配置」里（插件只有 Codex 支持，和多 Agent 的 MCP 列表不在一起）。
 * 从目录安装的插件按名字对上目录里的那一个，显示它的名字和图标；其它市场（如 openai-curated）显示配置键。
 */
import { useState } from "react";
import { Puzzle } from "lucide-react";
import { CxAddRow, CxBadge, CxCard, CxEmpty, CxRow, CxSection, CxStatusIcon } from "./cx-layout";
import { Action, Field, ManagementDrawer, Menu, MenuItem } from "./ManagementPages";
import { Switch } from "./gateway-ui";
import type { MarketPlugin } from "./plugin-market-model";
import { t, tf } from "./i18n";

export type PluginConfigEntry = { id: string; tomlBody: string; enabled: boolean };

/** 配置里写的启停：`enabled = false` 或旧写法 `disabled = true` 算停用，其余算启用（Codex 的默认）。 */
export function pluginConfigEnabled(body: string): boolean {
  if (/^\s*enabled\s*=\s*false\b/im.test(body)) return false;
  if (/^\s*disabled\s*=\s*true\b/im.test(body)) return false;
  return true;
}

/** 改启停：去掉原来的 enabled / disabled 行，在最前面写一行（它是根字段，必须在子表之前）。 */
export function pluginConfigWithEnabled(body: string, enabled: boolean): string {
  const rest = body.replace(/^\s*(?:enabled|disabled)\s*=\s*(?:true|false)\s*(?:#.*)?\r?\n?/gim, "");
  return `enabled = ${enabled}\n${rest}`;
}

/** 配置键 `<插件>@<市场>` 里的插件名；从目录装的插件按它对上目录条目。 */
export function pluginConfigName(id: string): string {
  const at = id.lastIndexOf("@");
  return at > 0 ? id.slice(0, at) : id;
}

export function PluginConfigPanel({ entries, plugins, onSave, busy: outerBusy }: {
  entries: PluginConfigEntry[];
  /** 目录里已安装的插件：用来给配置行配上名字。 */
  plugins: MarketPlugin[];
  /** 写一段配置；body 为 null 表示删除。 */
  onSave: (id: string, body: string | null) => Promise<boolean>;
  busy?: boolean;
}) {
  const [working, setWorking] = useState<string | null>(null);
  const [editor, setEditor] = useState<{ entry?: PluginConfigEntry } | null>(null);
  const [deleting, setDeleting] = useState<PluginConfigEntry | null>(null);
  const busy = outerBusy || working !== null;
  const run = async (key: string, task: () => Promise<boolean>) => {
    if (working) return false;
    setWorking(key);
    try {
      return await task();
    } finally {
      setWorking(null);
    }
  };
  const catalogOf = (entry: PluginConfigEntry) => {
    const name = pluginConfigName(entry.id);
    return plugins.find((plugin) => plugin.installed && plugin.name === name) ?? null;
  };
  return (
    <CxSection count={entries.length || undefined} title={t("本机插件配置")}>
      <p className="plugin-market-summary">{t("Codex config.toml 里的插件条目：从目录安装的，以及别处装的（如 Codex 自带市场）。关掉开关等于停用，不删文件。")}</p>
      <CxCard>
        {entries.length === 0 ? <CxEmpty title={t("config.toml 里还没有插件配置。")} /> : null}
        {entries.map((entry) => {
          const plugin = catalogOf(entry);
          const enabled = pluginConfigEnabled(entry.tomlBody);
          const title = plugin ? plugin.displayName || plugin.name : pluginConfigName(entry.id);
          return (
            <CxRow
              badges={plugin ? <CxBadge>{t("目录")}</CxBadge> : null}
              detail={<code className="cx-set-path" title={entry.id}>{entry.id}</code>}
              icon={<CxStatusIcon tone={enabled ? "ok" : "off"}><Puzzle aria-hidden="true" /></CxStatusIcon>}
              key={entry.id}
              muted={!enabled}
              title={title}
            >
              <Switch
                disabled={busy}
                label={tf("启用插件 {0}", [title])}
                on={enabled}
                onToggle={() => void run(`toggle:${entry.id}`, () => onSave(entry.id, pluginConfigWithEnabled(entry.tomlBody, !enabled)))}
                pending={working === `toggle:${entry.id}`}
              />
              <Menu label={t("插件配置操作")}>
                <MenuItem disabled={busy} onClick={() => setEditor({ entry })}>{t("编辑配置")}</MenuItem>
                <MenuItem danger disabled={busy} onClick={() => setDeleting(entry)}>{t("删除配置")}</MenuItem>
              </Menu>
            </CxRow>
          );
        })}
      </CxCard>
      <CxAddRow detail={t("手写一段 [plugins.\"<插件>@<市场>\"] 配置")} disabled={busy} onClick={() => setEditor({})} title={t("添加插件配置")} />
      {editor ? (
        <PluginConfigEditor
          busy={busy}
          entry={editor.entry}
          existing={entries.map((entry) => entry.id)}
          onClose={() => { if (!working) setEditor(null); }}
          onSave={(id, body) => run("edit", async () => {
            const saved = await onSave(id, body);
            if (saved) setEditor(null);
            return saved;
          })}
        />
      ) : null}
      {deleting ? (
        <ManagementDrawer
          footer={(
            <>
              <Action disabled={busy} onClick={() => setDeleting(null)}>{t("取消")}</Action>
              <Action danger disabled={busy} onClick={() => void run("delete", async () => {
                const saved = await onSave(deleting.id, null);
                if (saved) setDeleting(null);
                return saved;
              })}>{t("确认删除")}</Action>
            </>
          )}
          modal="confirm"
          onClose={() => { if (!working) setDeleting(null); }}
          title={t("删除插件配置")}
        >
          <strong className="mg-confirm-target">{deleting.id}</strong>
          <p className="mg-confirm-text">{t("只从 config.toml 里删掉这段配置，不删插件文件；从目录装的插件要卸载请在目录里点开它。")}</p>
        </ManagementDrawer>
      ) : null}
    </CxSection>
  );
}

function PluginConfigEditor({ entry, existing, busy, onClose, onSave }: {
  entry?: PluginConfigEntry;
  existing: string[];
  busy: boolean;
  onClose: () => void;
  onSave: (id: string, body: string) => Promise<boolean>;
}) {
  const [id, setId] = useState(entry?.id ?? "");
  const [body, setBody] = useState(entry?.tomlBody ?? "enabled = true\n");
  const key = id.trim();
  const duplicate = !entry && existing.includes(key);
  return (
    <ManagementDrawer
      dismissOnBackdrop={false}
      footer={(
        <>
          <Action disabled={busy} onClick={onClose}>{t("取消")}</Action>
          <Action disabled={busy || !key || duplicate} onClick={() => void onSave(key, body)} primary>{busy ? t("保存中…") : t("保存")}</Action>
        </>
      )}
      modal="form"
      onClose={onClose}
      title={entry ? t("编辑插件配置") : t("添加插件配置")}
    >
      <Field hint={duplicate ? t("已经有这个插件的配置，请直接编辑它。") : t("格式是 <插件>@<市场>，例如 github@openai-curated。")} label={t("配置键")}>
        <input aria-label={t("配置键")} disabled={Boolean(entry)} onChange={(event) => setId(event.target.value)} placeholder="github@openai-curated" value={id} />
      </Field>
      <Field hint={t("写在 [plugins.\"<配置键>\"] 下面的内容。")} label="TOML">
        <textarea aria-label="TOML" className="mg-code" onChange={(event) => setBody(event.target.value)} rows={10} spellCheck={false} value={body} />
      </Field>
    </ManagementDrawer>
  );
}
