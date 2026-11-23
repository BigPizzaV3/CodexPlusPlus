/**
 * 「Codex 插件」页的「已安装」：Codex config.toml 里的 `[plugins."<插件>@<市场>"]` 条目，一条一行。
 * 从目录装的按名字对上目录条目，显示它的图标、名字和简介（点名字看详情，有更新时给「更新」）；
 * 别处装的（如 Codex 自带的 openai-bundled）显示配置键。每行一个启用开关，菜单里编辑 / 删除这段配置；末尾一行添加。
 * 原来藏在「MCP 与 Skills」三点菜单的「管理插件配置」里（插件只有 Codex 支持，和多 Agent 的 MCP 列表不在一起）。
 */
import { useState, type ReactNode } from "react";
import { Download, LoaderCircle, PackageOpen } from "lucide-react";
import { CxAddRow, CxBadge, CxCard, CxEmpty } from "./cx-layout";
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

/** 「已安装」的数量：配置里的条目，加上目录里装了却没有配置的（少见，比如配置被手动删了）。 */
export function installedPluginCount(entries: PluginConfigEntry[], plugins: MarketPlugin[]): number {
  const names = new Set(entries.map((entry) => pluginConfigName(entry.id)));
  return entries.length + plugins.filter((plugin) => plugin.installed && !names.has(plugin.name)).length;
}

export function PluginConfigPanel({ entries, plugins, onSave, busy: outerBusy, query = "", icon, onOpenDetails, onUpdate, updatingId }: {
  entries: PluginConfigEntry[];
  /** 目录里的插件：用来给配置行配上名字、图标和简介。 */
  plugins: MarketPlugin[];
  /** 写一段配置；body 为 null 表示删除。 */
  onSave: (id: string, body: string | null) => Promise<boolean>;
  busy?: boolean;
  /** 页面的搜索词：按名字、配置键和简介过滤。 */
  query?: string;
  /** 目录插件的图标（和目录列表同一个组件）。 */
  icon?: (plugin: MarketPlugin) => ReactNode;
  onOpenDetails?: (plugin: MarketPlugin, trigger: HTMLElement) => void;
  onUpdate?: (plugin: MarketPlugin) => void;
  /** 正在安装 / 更新的目录插件。 */
  updatingId?: string | null;
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
  // 目录里装了、配置里却没有的（配置被手动删了）：照样列出来，只是没有开关。
  const configured = new Set(entries.map((entry) => pluginConfigName(entry.id)));
  const orphans = plugins.filter((plugin) => plugin.installed && !configured.has(plugin.name));
  const needle = query.trim().toLowerCase();
  const hit = (...values: Array<string | undefined>) => !needle || values.some((value) => value?.toLowerCase().includes(needle));
  const shown = entries.filter((entry) => {
    const plugin = catalogOf(entry);
    return hit(entry.id, plugin?.displayName, plugin?.name, plugin?.shortDescription, plugin?.description);
  });
  const shownOrphans = orphans.filter((plugin) => hit(plugin.displayName, plugin.name, plugin.shortDescription));
  const updateButton = (plugin: MarketPlugin, title: string) => plugin.updateAvailable && onUpdate ? (
    updatingId === plugin.id
      ? <span aria-label={t("安装中…")} className="plugin-market-row-action" role="status"><LoaderCircle aria-hidden="true" className="plugin-market-spinner" /></span>
      : <button aria-label={tf("更新插件 {0}", [title])} className="cx-icon-button plugin-market-row-action" disabled={busy} onClick={() => onUpdate(plugin)} title={t("更新插件")} type="button"><Download aria-hidden="true" /></button>
  ) : null;
  const genericIcon = <span aria-hidden="true" className="plugin-market-icon"><PackageOpen /></span>;
  // 和目录列表同一种行：图标、名字、一句话简介；右边是开关和菜单（目录列表那里是 + / 对勾）。
  // 目录插件点开看详情；别处装的点开编辑配置。
  const rowMain = (title: string, description: string, iconNode: ReactNode, onOpen: ((trigger: HTMLElement) => void) | null, badge?: ReactNode) => {
    const body = (
      <>
        {iconNode}
        <span className="plugin-market-row-text">
          <span className="plugin-market-row-title"><strong title={title}>{title}</strong>{badge}</span>
          <span className="plugin-market-row-description" title={description}>{description}</span>
        </span>
      </>
    );
    return onOpen ? (
      <button aria-label={tf("查看插件 {0} 的详情", [title])} className="plugin-market-row-main" onClick={(event) => onOpen(event.currentTarget)} type="button">{body}</button>
    ) : <span className="plugin-market-row-main">{body}</span>;
  };
  return (
    <section className="plugin-config-panel plugin-market-shelf">
      <p className="plugin-market-summary">{t("Codex config.toml 里的插件：从目录安装的，以及别处装的（如 Codex 自带的）。关掉开关等于停用，不删文件。")}</p>
      {shown.length === 0 && shownOrphans.length === 0 ? (
        <CxCard><CxEmpty title={needle ? t("没有匹配的插件。") : t("还没有安装插件。")} /></CxCard>
      ) : (
        <ul className="plugin-market-rows">
          {shown.map((entry) => {
            const plugin = catalogOf(entry);
            const enabled = pluginConfigEnabled(entry.tomlBody);
            const title = plugin ? plugin.displayName || plugin.name : pluginConfigName(entry.id);
            const description = plugin ? plugin.shortDescription || plugin.description || entry.id : entry.id;
            const open = plugin && onOpenDetails
              ? (trigger: HTMLElement) => onOpenDetails(plugin, trigger)
              : () => setEditor({ entry });
            return (
              <li className="plugin-market-row" data-disabled={!enabled || undefined} key={entry.id}>
                {rowMain(title, description, plugin && icon ? icon(plugin) : genericIcon, open)}
                {plugin ? updateButton(plugin, title) : null}
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
              </li>
            );
          })}
          {shownOrphans.map((plugin) => {
            const title = plugin.displayName || plugin.name;
            return (
              <li className="plugin-market-row" data-disabled key={`orphan:${plugin.id}`}>
                {rowMain(
                  title,
                  plugin.shortDescription || plugin.description || plugin.name,
                  icon ? icon(plugin) : genericIcon,
                  onOpenDetails ? (trigger) => onOpenDetails(plugin, trigger) : null,
                  <CxBadge tone="warn" title={t("插件文件在，但 config.toml 里没有它的配置，Codex 不会加载；在目录里点开它重新安装即可。")}>{t("没有配置")}</CxBadge>,
                )}
                {updateButton(plugin, title)}
              </li>
            );
          })}
        </ul>
      )}
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
    </section>
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
