import i18n from "../../i18n";
import { CHAT_UPSTREAM_BASE_URL_KEY, DEFAULT_RELAY_PROFILE_ID, PROTOCOL_PROXY_BASE_URL, STORAGE_KEYS, defaultSettings, emptyContextSelection, routes, type AdItem, type BackendSettings, type CodexContextEntries, type CodexContextEntry, type ContextKind, type OverviewResult, type RelayContextSelection, type RelayMode, type RelayProfile, type RelayProtocol, type RelayResult, type Route, type Status, type Theme } from "./model";
export function isExpiredAd(ad: AdItem) {
    if (!ad.expires_at)
        return false;
    const expiresAt = Date.parse(ad.expires_at);
    return Number.isFinite(expiresAt) && expiresAt < Date.now();
}
export function routeTitleKey(route: Route) {
    return routes.find((item) => item.id === route)?.labelKey ?? "nav.overview";
}
export function routeSubtitleKey(route: Route) {
    const keys: Record<Route, string> = {
        overview: "route.overview.subtitle",
        relay: "route.relay.subtitle",
        sessions: "route.sessions.subtitle",
        context: "route.context.subtitle",
        enhance: "route.enhance.subtitle",
        userScripts: "route.userScripts.subtitle",
        recommendations: "route.recommendations.subtitle",
        maintenance: "route.maintenance.subtitle",
        about: "route.about.subtitle",
        settings: "route.settings.subtitle",
    };
    return keys[route];
}
export const contextKindOptions: Array<{
    kind: ContextKind;
    tableName: string;
}> = [
    { kind: "mcp", tableName: "mcp_servers" },
    { kind: "skill", tableName: "skills" },
    { kind: "plugin", tableName: "plugins" },
];
const contextKindLabelKeys: Record<ContextKind, string> = {
    mcp: "contextKindLabel.mcp",
    skill: "contextKindLabel.skill",
    plugin: "contextKindLabel.plugin",
};
export function contextKindLabel(kind: ContextKind) {
    return i18n.t(contextKindLabelKeys[kind] ?? "contextKindLabel.fallback");
}
export function contextEntriesFromSettings(settings: BackendSettings): CodexContextEntries {
    const commonConfig = normalizeDuplicateTomlTables(settings.relayContextConfigContents || "");
    return {
        mcpServers: parseContextEntries(commonConfig, "mcp", "mcp_servers"),
        skills: parseContextEntries(commonConfig, "skill", "skills"),
        plugins: parseContextEntries(commonConfig, "plugin", "plugins"),
    };
}
export function contextEntriesWithLiveEntries(settings: BackendSettings, liveEntries: CodexContextEntries | null): CodexContextEntries {
    const commonEntries = contextEntriesFromSettings(settings);
    if (!liveEntries)
        return commonEntries;
    const liveByKind: Record<ContextKind, Map<string, CodexContextEntry>> = {
        mcp: new Map(liveEntries.mcpServers.map((entry) => [entry.id, entry])),
        skill: new Map(liveEntries.skills.map((entry) => [entry.id, entry])),
        plugin: new Map(liveEntries.plugins.map((entry) => [entry.id, entry])),
    };
    return {
        mcpServers: mergeLiveContextEntries(commonEntries.mcpServers, liveByKind.mcp),
        skills: mergeLiveContextEntries(commonEntries.skills, liveByKind.skill),
        plugins: mergeLiveContextEntries(commonEntries.plugins, liveByKind.plugin),
    };
}
export function mergeLiveContextEntries(entries: CodexContextEntry[], liveEntries: Map<string, CodexContextEntry>): CodexContextEntry[] {
    const uniqueEntries = dedupeContextEntryList(entries);
    const merged = uniqueEntries.map((entry) => {
        const live = liveEntries.get(entry.id);
        return withLiveEntryState(entry, live);
    });
    const knownIds = new Set(uniqueEntries.map((entry) => entry.id));
    for (const liveEntry of liveEntries.values()) {
        if (!knownIds.has(liveEntry.id))
            merged.push(liveEntry);
    }
    return merged;
}
export function withLiveEntryState(entry: CodexContextEntry, live?: CodexContextEntry): CodexContextEntry {
    return live ? { ...entry, enabled: live.enabled } : { ...entry, enabled: false };
}
export function contextEntriesForProfile(settings: BackendSettings, _profile: RelayProfile): CodexContextEntries {
    return contextEntriesFromSettings(settings);
}
export function contextEntriesFromConfig(configContents: string): CodexContextEntries {
    return {
        mcpServers: parseContextEntries(configContents, "mcp", "mcp_servers"),
        skills: parseContextEntries(configContents, "skill", "skills"),
        plugins: parseContextEntries(configContents, "plugin", "plugins"),
    };
}
export function mergeContextEntries(primary: CodexContextEntries, secondary: CodexContextEntries): CodexContextEntries {
    return {
        mcpServers: mergeContextEntryList(primary.mcpServers, secondary.mcpServers),
        skills: mergeContextEntryList(primary.skills, secondary.skills),
        plugins: mergeContextEntryList(primary.plugins, secondary.plugins),
    };
}
export function mergeContextEntryList(primary: CodexContextEntry[], secondary: CodexContextEntry[]): CodexContextEntry[] {
    return dedupeContextEntryList([...primary, ...secondary]);
}
export function dedupeContextEntryList(entries: CodexContextEntry[]): CodexContextEntry[] {
    const byId = new Map<string, CodexContextEntry>();
    for (const entry of entries) {
        byId.set(entry.id, entry);
    }
    return Array.from(byId.values());
}
export function parseContextEntries(commonConfig: string, kind: ContextKind, tableName: string): CodexContextEntry[] {
    const anyHeaderPattern = /^\s*\[[^\]]+\]\s*$/;
    const entries = new Map<string, CodexContextEntry>();
    let currentId: string | null = null;
    let body: string[] = [];
    const flush = () => {
        if (!currentId)
            return;
        const tomlBody = ensureTrailingNewline(body.join("\n").trimEnd());
        entries.set(currentId, {
            id: currentId,
            kind,
            title: currentId,
            summary: contextEntrySummary(tomlBody),
            tomlBody,
            enabled: contextEntryEnabled(tomlBody),
        });
    };
    for (const line of commonConfig.split(/\r?\n/)) {
        const path = tomlTablePathFromLine(line);
        if (path?.[0] === tableName && path.length >= 2) {
            const id = path[1];
            if (currentId === id && path.length > 2) {
                body.push(`[${path.slice(2).map(tomlKey).join(".")}]`);
                continue;
            }
            flush();
            currentId = id;
            body = [];
            continue;
        }
        if (currentId && anyHeaderPattern.test(line)) {
            flush();
            currentId = null;
            body = [];
            continue;
        }
        if (currentId)
            body.push(line);
    }
    flush();
    return Array.from(entries.values());
}
export function tomlTablePathFromLine(line: string): string[] | null {
    const match = /^\s*\[([^\]]+)\]\s*$/.exec(line);
    if (!match)
        return null;
    return parseTomlDottedPath(match[1].trim());
}
export function parseTomlDottedPath(path: string): string[] | null {
    const parts: string[] = [];
    let current = "";
    let quote: '"' | "'" | null = null;
    let escaping = false;
    for (const char of path) {
        if (quote) {
            if (quote === '"' && escaping) {
                current += char;
                escaping = false;
            }
            else if (quote === '"' && char === "\\") {
                escaping = true;
            }
            else if (char === quote) {
                quote = null;
            }
            else {
                current += char;
            }
            continue;
        }
        if (char === '"' || char === "'") {
            quote = char;
            continue;
        }
        if (char === ".") {
            if (!current.trim())
                return null;
            parts.push(current.trim());
            current = "";
            continue;
        }
        current += char;
    }
    if (quote || escaping || !current.trim())
        return null;
    parts.push(current.trim());
    return parts;
}
export function contextEntrySummary(tomlBody: string) {
    return tomlBody
        .split(/\r?\n/)
        .map((line) => line.trim())
        .find((line) => line && !line.startsWith("#") && !/^enabled\s*=/.test(line))
        ?.slice(0, 96) ?? "";
}
export function contextEntryEnabled(tomlBody: string) {
    return !tomlBody.split(/\r?\n/).some((line) => /^\s*enabled\s*=\s*false\s*(#.*)?$/i.test(line));
}
export function setContextEntryEnabled(tomlBody: string, enabled: boolean) {
    const lines = tomlBody.trimEnd().split(/\r?\n/);
    const nextValue = `enabled = ${enabled ? "true" : "false"}`;
    let replaced = false;
    const next = lines.map((line) => {
        if (/^\s*enabled\s*=/.test(line)) {
            replaced = true;
            return nextValue;
        }
        return line;
    });
    if (!replaced)
        next.unshift(nextValue);
    return ensureTrailingNewline(next.join("\n").trimEnd());
}
export function ensureTrailingNewline(value: string) {
    return value.trim() ? `${value}\n` : "";
}
export function unquoteTomlKey(key: string) {
    if (key.length >= 2 && ((key.startsWith('"') && key.endsWith('"')) || (key.startsWith("'") && key.endsWith("'")))) {
        return key.slice(1, -1);
    }
    return key;
}
export function contextEntriesByKind(entries: CodexContextEntries, kind: ContextKind): CodexContextEntry[] {
    if (kind === "mcp")
        return dedupeContextEntryList(entries.mcpServers);
    if (kind === "skill")
        return dedupeContextEntryList(entries.skills);
    return dedupeContextEntryList(entries.plugins);
}
export function configHasCodexGoalsFeature(configContents: string): boolean {
    let inFeatures = false;
    for (const line of configContents.split(/\r?\n/)) {
        const trimmed = line.trim();
        if (/^\[features\]$/.test(trimmed)) {
            inFeatures = true;
            continue;
        }
        if (inFeatures && /^\[[^\]]+\]$/.test(trimmed)) {
            inFeatures = false;
        }
        if (inFeatures && /^goals\s*=\s*true\b/.test(trimmed)) {
            return true;
        }
    }
    return false;
}
export function setCodexGoalsFeatureInConfig(configContents: string, enabled: boolean): string {
    const lines = configContents.split(/\r?\n/);
    const next: string[] = [];
    let inFeatures = false;
    let sawFeatures = false;
    let featuresHasGoals = false;
    const maybeInsertGoals = () => {
        if (enabled && sawFeatures && !featuresHasGoals) {
            next.push("goals = true");
            featuresHasGoals = true;
        }
    };
    for (const line of lines) {
        const trimmed = line.trim();
        if (/^\[features\]$/.test(trimmed)) {
            if (inFeatures)
                maybeInsertGoals();
            inFeatures = true;
            sawFeatures = true;
            featuresHasGoals = false;
            next.push(line);
            continue;
        }
        if (inFeatures && /^\[[^\]]+\]$/.test(trimmed)) {
            maybeInsertGoals();
            inFeatures = false;
        }
        if (inFeatures && /^goals\s*=/.test(trimmed)) {
            if (enabled && !featuresHasGoals) {
                next.push("goals = true");
                featuresHasGoals = true;
            }
            continue;
        }
        next.push(line);
    }
    if (inFeatures)
        maybeInsertGoals();
    if (enabled && !sawFeatures) {
        const trimmed = ensureTrailingNewline(next.join("\n").trimEnd());
        return joinTomlSections([trimmed, "[features]\ngoals = true"]);
    }
    return ensureTrailingNewline(next.join("\n").trimEnd());
}
export function effectiveRelayConfigPreview(profile: RelayProfile, settings: BackendSettings, contextProfile = profile): string {
    const entries = contextEntriesForProfile(settings, contextProfile);
    const isolatedConfig = stripContextEntriesFromConfig(profile.configContents, entries);
    const configWithLimits = applyContextLimitPreview(isolatedConfig, profile);
    return joinTomlSectionsRootFirst([configWithLimits, settings.relayCommonConfigContents || "", selectedContextConfigToml(entries)]);
}
export function selectedContextConfigToml(entries: CodexContextEntries): string {
    const sections: string[] = [];
    for (const option of contextKindOptions) {
        for (const entry of dedupeContextEntryList(contextEntriesByKind(entries, option.kind))) {
            if (!entry.enabled)
                continue;
            sections.push(contextEntryToTomlSection(option.tableName, entry));
        }
    }
    return ensureTrailingNewline(sections.join("\n\n"));
}
export function allContextConfigToml(entries: CodexContextEntries): string {
    const sections: string[] = [];
    for (const option of contextKindOptions) {
        for (const entry of dedupeContextEntryList(contextEntriesByKind(entries, option.kind))) {
            sections.push(contextEntryToTomlSection(option.tableName, entry));
        }
    }
    return ensureTrailingNewline(sections.join("\n\n"));
}
export function contextEntryToTomlSection(tableName: string, entry: CodexContextEntry): string {
    const parentHeader = `[${tableName}.${tomlKey(entry.id)}]`;
    const body = entry.tomlBody
        .trimEnd()
        .split(/\r?\n/)
        .map((line) => relativeContextSubtableToAbsolute(line, tableName, entry.id))
        .join("\n");
    return `${parentHeader}\n${body}`;
}
export function relativeContextSubtableToAbsolute(line: string, tableName: string, id: string): string {
    const match = /^\s*\[([^\]]+)\]\s*$/.exec(line);
    if (!match)
        return line;
    const subtable = match[1].trim();
    if (!subtable || subtable.includes("."))
        return line;
    return `[${tableName}.${tomlKey(id)}.${tomlKey(subtable)}]`;
}
export function syncLiveConfigContextState(liveConfigContents: string, settings: BackendSettings): string {
    const entries = contextEntriesFromSettings(settings);
    const withoutContext = stripAllContextEntriesFromConfig(liveConfigContents);
    return joinTomlSectionsRootFirst([withoutContext, selectedContextConfigToml(entries)]);
}
export function relayCombinedCommonConfig(settings: BackendSettings): string {
    return joinTomlSectionsRootFirst([settings.relayCommonConfigContents || "", settings.relayContextConfigContents || ""]);
}
export function splitContextConfigText(configContents: string): {
    common: string;
    context: string;
} {
    const entries = contextEntriesFromConfig(configContents);
    return {
        common: stripContextEntriesFromConfig(configContents, entries),
        context: allContextConfigToml(entries),
    };
}
export function stripContextEntriesFromConfig(configContents: string, entries: CodexContextEntries): string {
    const knownIds: Record<ContextKind, Set<string>> = {
        mcp: new Set(entries.mcpServers.map((entry) => entry.id)),
        skill: new Set(entries.skills.map((entry) => entry.id)),
        plugin: new Set(entries.plugins.map((entry) => entry.id)),
    };
    const lines = configContents.split(/\r?\n/);
    const kept: string[] = [];
    let skipping = false;
    for (const line of lines) {
        const contextHeader = contextHeaderFromLine(line);
        if (contextHeader) {
            skipping = knownIds[contextHeader.kind].has(contextHeader.id);
        }
        else if (/^\s*\[[^\]]+\]\s*$/.test(line)) {
            skipping = false;
        }
        if (!skipping)
            kept.push(line);
    }
    return ensureTrailingNewline(kept.join("\n").trimEnd());
}
export function stripAllContextEntriesFromConfig(configContents: string): string {
    const lines = configContents.split(/\r?\n/);
    const kept: string[] = [];
    let skipping = false;
    for (const line of lines) {
        const contextHeader = contextHeaderFromLine(line);
        if (contextHeader) {
            skipping = true;
        }
        else if (/^\s*\[[^\]]+\]\s*$/.test(line)) {
            skipping = false;
        }
        if (!skipping)
            kept.push(line);
    }
    return ensureTrailingNewline(kept.join("\n").trimEnd());
}
export function stripCommonConfigTextFallback(configContents: string, commonConfig: string): string {
    const anchors = commonConfigAnchors(commonConfig);
    if (!anchors.rootKeys.size && !anchors.tableHeaders.size)
        return ensureTrailingNewline(configContents.trimEnd());
    const kept: string[] = [];
    let skippingTable = false;
    for (const line of configContents.split(/\r?\n/)) {
        const trimmed = line.trim();
        if (/^\[[^\]]+\]$/.test(trimmed)) {
            skippingTable = anchors.tableHeaders.has(trimmed);
            if (skippingTable)
                continue;
        }
        if (skippingTable)
            continue;
        const key = tomlRootKeyFromLine(trimmed);
        if (key && anchors.rootKeys.has(key))
            continue;
        kept.push(line);
    }
    return ensureTrailingNewline(kept.join("\n").trimEnd());
}
export function commonConfigAnchors(commonConfig: string): {
    rootKeys: Set<string>;
    tableHeaders: Set<string>;
} {
    const rootKeys = new Set<string>();
    const tableHeaders = new Set<string>();
    let inRoot = true;
    for (const line of commonConfig.split(/\r?\n/)) {
        const trimmed = line.trim();
        if (/^\[[^\]]+\]$/.test(trimmed)) {
            inRoot = false;
            tableHeaders.add(trimmed);
            continue;
        }
        if (inRoot) {
            const key = tomlRootKeyFromLine(trimmed);
            if (key)
                rootKeys.add(key);
        }
    }
    return { rootKeys, tableHeaders };
}
export function tomlRootKeyFromLine(line: string): string | null {
    if (!line || line.startsWith("#"))
        return null;
    const index = line.indexOf("=");
    if (index < 0)
        return null;
    const key = line.slice(0, index).trim();
    return key || null;
}
export function contextHeaderFromLine(line: string): {
    kind: ContextKind;
    id: string;
} | null {
    const path = tomlTablePathFromLine(line);
    if (!path || path.length !== 2)
        return null;
    const option = contextKindOptions.find((item) => item.tableName === path[0]);
    return option ? { kind: option.kind, id: path[1] } : null;
}
export function applyContextLimitPreview(configContents: string, profile: RelayProfile): string {
    const replacements: Array<[
        string,
        string
    ]> = [
        ["model_context_window", profile.contextWindow],
        ["model_auto_compact_token_limit", profile.autoCompactLimit],
    ];
    let lines = configContents.split(/\r?\n/);
    for (const [key, value] of replacements) {
        const trimmed = value.trim();
        if (!trimmed)
            continue;
        let replaced = false;
        lines = lines.map((line) => {
            if (!replaced && new RegExp(`^\\s*${key}\\s*=`).test(line)) {
                replaced = true;
                return `${key} = ${trimmed}`;
            }
            return line;
        });
        if (!replaced) {
            const firstTable = lines.findIndex((line) => /^\s*\[[^\]]+\]\s*$/.test(line));
            const insertAt = firstTable >= 0 ? firstTable : lines.length;
            lines.splice(insertAt, 0, `${key} = ${trimmed}`);
        }
    }
    return ensureTrailingNewline(lines.join("\n").trimEnd());
}
export function removeRootTomlKey(contents: string, key: string): string {
    const lines: string[] = [];
    let inRoot = true;
    for (const line of contents.split(/\r?\n/)) {
        if (/^\s*\[[^\]]+\]\s*$/.test(line))
            inRoot = false;
        if (inRoot && new RegExp(`^\\s*${key}\\s*=`).test(line))
            continue;
        lines.push(line);
    }
    return ensureTrailingNewline(lines.join("\n").trimEnd());
}
export function joinTomlSections(sections: string[]): string {
    return ensureTrailingNewline(sections
        .map((section) => section.trim())
        .filter(Boolean)
        .join("\n\n"));
}
export function joinTomlSectionsRootFirst(sections: string[]): string {
    const rootParts: string[] = [];
    const tableParts: string[] = [];
    for (const section of sections) {
        const { root, tables } = splitTomlRootAndTables(section);
        if (root.trim())
            rootParts.push(root.trim());
        if (tables.trim())
            tableParts.push(tables.trim());
    }
    return normalizeDuplicateTomlTables(joinTomlSections([...dedupeTomlRootLines(rootParts), ...tableParts]));
}
export function normalizeDuplicateTomlTables(contents: string): string {
    const seenHeaders = new Set<string>();
    const kept: string[] = [];
    let skipping = false;
    for (const line of contents.split(/\r?\n/)) {
        const trimmed = line.trim();
        if (/^\[[^\]]+\]$/.test(trimmed)) {
            skipping = seenHeaders.has(trimmed);
            seenHeaders.add(trimmed);
            if (skipping)
                continue;
        }
        if (!skipping)
            kept.push(line);
    }
    return ensureTrailingNewline(kept.join("\n").trimEnd());
}
export function dedupeTomlRootLines(rootParts: string[]): string[] {
    const rootLines = rootParts
        .join("\n")
        .split(/\r?\n/)
        .map((line) => line.trimEnd());
    const rootSeen = new Set<string>();
    const kept: string[] = [];
    for (let index = rootLines.length - 1; index >= 0; index -= 1) {
        const line = rootLines[index];
        const key = tomlRootKeyFromLine(line.trim());
        if (key) {
            if (rootSeen.has(key))
                continue;
            rootSeen.add(key);
        }
        kept.push(line);
    }
    const normalized = kept.reverse().join("\n").trim();
    return normalized ? [normalized] : [];
}
export function splitTomlRootAndTables(section: string): {
    root: string;
    tables: string;
} {
    const lines = section.trim().split(/\r?\n/);
    const firstTable = lines.findIndex((line) => /^\s*\[[^\]]+\]\s*$/.test(line));
    if (firstTable < 0)
        return { root: lines.join("\n"), tables: "" };
    return {
        root: lines.slice(0, firstTable).join("\n"),
        tables: lines.slice(firstTable).join("\n"),
    };
}
export function tomlKey(key: string): string {
    return /^[A-Za-z0-9_-]+$/.test(key) ? key : `"${tomlString(key)}"`;
}
export function contextSelectionIds(selection: RelayContextSelection, kind: ContextKind): string[] {
    if (kind === "mcp")
        return selection.mcpServers;
    if (kind === "skill")
        return selection.skills;
    return selection.plugins;
}
export function setContextSelectionId(selection: RelayContextSelection, kind: ContextKind, id: string, checked: boolean): RelayContextSelection {
    const next = {
        mcpServers: [...selection.mcpServers],
        skills: [...selection.skills],
        plugins: [...selection.plugins],
    };
    const list = contextSelectionIds(next, kind);
    const normalizedId = id.trim();
    const exists = list.includes(normalizedId);
    if (checked && normalizedId && !exists)
        list.push(normalizedId);
    if (!checked && exists)
        list.splice(list.indexOf(normalizedId), 1);
    return next;
}
export function removeContextSelectionFromSettings(settings: BackendSettings, kind: ContextKind, id: string): BackendSettings {
    return {
        ...settings,
        relayProfiles: settings.relayProfiles.map((profile) => ({
            ...profile,
            contextSelection: setContextSelectionId(profile.contextSelection, kind, id, false),
        })),
    };
}
export function contextSelectionForAllEntries(settings: BackendSettings): RelayContextSelection {
    const entries = contextEntriesFromSettings(settings);
    return {
        mcpServers: entries.mcpServers.map((entry) => entry.id),
        skills: entries.skills.map((entry) => entry.id),
        plugins: entries.plugins.map((entry) => entry.id),
    };
}
export function relayProfileSourceLabel(profile: RelayProfile) {
    return profile.linkedCcsProviderId ? i18n.t("relaySourceLabel.ccs") : i18n.t("relaySourceLabel.local");
}
export function relayProfileEditorStatus(profile: RelayProfile, form: BackendSettings, isNew: boolean) {
    if (isNew)
        return i18n.t("relayProfileEditorStatus.new");
    if (!form.relayProfilesEnabled)
        return i18n.t("relayProfileEditorStatus.disabled");
    if (profile.linkedCcsProviderId && form.ccsLinkEnabled)
        return i18n.t("relayProfileEditorStatus.linked");
    if (profile.linkedCcsProviderId)
        return i18n.t("relayProfileEditorStatus.linkedNoWrite");
    return profile.id === form.activeRelayId ? i18n.t("relayProfileEditorStatus.active") : i18n.t("relayProfileEditorStatus.edited");
}
export function providerInitial(name: string) {
    const trimmed = (name || i18n.t("providerInitial.fallback")).trim();
    return Array.from(trimmed)[0]?.toUpperCase() || i18n.t("providerInitial.initial");
}
export function statusLabel(status: string) {
    const labels: Record<string, string> = {
        found: i18n.t("statusLabel.found"),
        missing: i18n.t("statusLabel.missing"),
        installed: i18n.t("statusLabel.installed"),
        ok: i18n.t("statusLabel.ok"),
        running: i18n.t("statusLabel.running"),
        failed: i18n.t("statusLabel.failed"),
        archived: i18n.t("statusLabel.archived"),
        accepted: i18n.t("statusLabel.accepted"),
        not_checked: i18n.t("statusLabel.notChecked"),
        not_implemented: i18n.t("statusLabel.notImplemented"),
        disabled: i18n.t("statusLabel.disabled"),
        unknown: i18n.t("statusLabel.unknown"),
    };
    return labels[status] ?? status;
}
export function statusClass(status: string) {
    if (["found", "installed", "ok", "running"].includes(status))
        return "good";
    if (["failed", "missing"].includes(status))
        return "bad";
    return "warn";
}
export function isSuccessStatus(status?: Status) {
    return status === "ok" || status === "accepted";
}
export function healthItems(overview: OverviewResult | null) {
    return [
        {
            title: i18n.t("health.codexApp"),
            status: overview?.codex_app.status ?? "not_checked",
            ok: overview?.codex_app.status === "found",
            detail: overview?.codex_app.path || i18n.t("health.codexAppDetail"),
        },
        {
            title: i18n.t("health.silentEntry"),
            status: overview?.silent_shortcut.status ?? "not_checked",
            ok: overview?.silent_shortcut.status === "installed",
            detail: overview?.silent_shortcut.path || i18n.t("health.silentEntryDetail"),
        },
        {
            title: i18n.t("health.managerEntry"),
            status: overview?.management_shortcut.status ?? "not_checked",
            ok: overview?.management_shortcut.status === "installed",
            detail: overview?.management_shortcut.path || i18n.t("health.managerEntryDetail"),
        },
    ];
}
export function normalizeSettings(settings: BackendSettings): BackendSettings {
    const splitCommon = splitContextConfigText(settings.relayCommonConfigContents || "");
    const relayCommonConfigContents = splitCommon.common;
    const relayContextConfigContents = joinTomlSectionsRootFirst([
        settings.relayContextConfigContents || "",
        splitCommon.context,
    ]);
    const defaultContextSelection = contextSelectionForAllEntries({
        ...settings,
        relayCommonConfigContents,
        relayContextConfigContents,
    });
    const profiles = settings.relayProfiles?.length
        ? settings.relayProfiles.map((profile) => normalizeRelayProfile(profile, defaultContextSelection))
        : [
            {
                id: settings.activeRelayId || DEFAULT_RELAY_PROFILE_ID,
                linkedCcsProviderId: "",
                name: i18n.t("createRelayProfile.defaultProfileName"),
                model: "",
                baseUrl: settings.relayBaseUrl || defaultSettings.relayBaseUrl,
                upstreamBaseUrl: settings.relayBaseUrl || defaultSettings.relayBaseUrl,
                apiKey: settings.relayApiKey || "",
                protocol: "responses" as RelayProtocol,
                relayMode: "official" as RelayMode,
                officialMixApiKey: false,
                testModel: "",
                configContents: "",
                authContents: "",
                useCommonConfig: true,
                contextSelection: defaultContextSelection,
                contextSelectionInitialized: true,
                contextWindow: "",
                autoCompactLimit: "",
                modelList: "",
                userAgent: "",
            },
        ];
    const activeRelayId = profiles.some((profile) => profile.id === settings.activeRelayId)
        ? settings.activeRelayId
        : profiles[0]?.id || DEFAULT_RELAY_PROFILE_ID;
    return syncLegacyRelayFields({
        ...defaultSettings,
        ...settings,
        relayProfilesEnabled: settings.relayProfilesEnabled !== false,
        ccsLinkEnabled: settings.ccsLinkEnabled === true,
        relayCommonConfigContents,
        relayContextConfigContents,
        relayProfiles: profiles,
        activeRelayId,
    });
}
export function codexExtraArgsToInput(args: string[] | undefined) {
    return (args ?? []).join("\n");
}
export function inputToCodexExtraArgs(value: string) {
    return value === "" ? [] : value.split(/\r?\n/);
}
export function normalizeRelayProfile(profile: RelayProfile, defaultContextSelection = emptyContextSelection()): RelayProfile {
    const legacyMixedApi = profile.relayMode === "mixedApi";
    let normalized: RelayProfile = {
        ...profile,
        linkedCcsProviderId: profile.linkedCcsProviderId || "",
        model: profile.model || "",
        baseUrl: profile.baseUrl || defaultSettings.relayBaseUrl,
        upstreamBaseUrl: profile.upstreamBaseUrl || profile.baseUrl || "",
        apiKey: profile.apiKey || "",
        protocol: profile.protocol === "chatCompletions" ? "chatCompletions" : "responses",
        relayMode: normalizeRelayMode(profile.relayMode),
        officialMixApiKey: profile.officialMixApiKey === true || legacyMixedApi,
        testModel: profile.testModel || "",
        configContents: profile.configContents || "",
        authContents: profile.authContents || "",
        useCommonConfig: profile.useCommonConfig !== false,
        contextSelection: profile.contextSelectionInitialized
            ? normalizeContextSelection(profile.contextSelection)
            : normalizeContextSelection(undefined, defaultContextSelection),
        contextSelectionInitialized: true,
        contextWindow: profile.contextWindow || "",
        autoCompactLimit: profile.autoCompactLimit || "",
        modelList: profile.modelList || "",
        userAgent: profile.userAgent || "",
    };
    return deriveRelayProfileFromFiles(normalized);
}
export function activeRelayProfile(settings: BackendSettings): RelayProfile {
    return (settings.relayProfiles.find((profile) => profile.id === settings.activeRelayId) ||
        settings.relayProfiles[0] ||
        defaultSettings.relayProfiles[0]);
}
export function relayProtocolLabel(protocol: RelayProtocol): string {
    return protocol === "chatCompletions" ? i18n.t("relayProtocol.chatCompletions") : i18n.t("relayProtocol.responsesApi");
}
export function normalizeRelayMode(mode: RelayMode | undefined): RelayMode {
    if (mode === "pureApi")
        return mode;
    return "official";
}
export function normalizeContextSelection(selection?: Partial<RelayContextSelection>, fallback: RelayContextSelection = emptyContextSelection()): RelayContextSelection {
    if (!selection) {
        return {
            mcpServers: [...fallback.mcpServers],
            skills: [...fallback.skills],
            plugins: [...fallback.plugins],
        };
    }
    return {
        mcpServers: Array.isArray(selection?.mcpServers) ? selection.mcpServers.map(String) : [],
        skills: Array.isArray(selection?.skills) ? selection.skills.map(String) : [],
        plugins: Array.isArray(selection?.plugins) ? selection.plugins.map(String) : [],
    };
}
export function relayModeLabel(mode: RelayMode): string {
    if (mode === "pureApi")
        return i18n.t("relayModeLabel.pureApi");
    return i18n.t("relayModeLabel.official");
}
export function relayProfileConfigBrief(profile: RelayProfile): string {
    if (profile.relayMode === "official")
        return profile.officialMixApiKey ? i18n.t("relayProfileConfigBrief.mixApiKey") : i18n.t("relayProfileConfigBrief.noApiFile");
    return profile.baseUrl || i18n.t("relayProfileConfigBrief.noUrl");
}
export function relayProfileModeHelp(profile: RelayProfile): string {
    if (profile.relayMode === "official") {
        if (profile.officialMixApiKey) {
            return i18n.t("relayProfileModeHelp.mixOfficial");
        }
        return i18n.t("relayProfileModeHelp.official");
    }
    if (profile.relayMode === "pureApi") {
        return i18n.t("relayProfileModeHelp.pureApi");
    }
    return i18n.t("relayProfileModeHelp.mixOfficial");
}
export function relayProfileReadinessText(profile: RelayProfile, relay: RelayResult | null): string {
    if (profile.relayMode === "official") {
        if (profile.officialMixApiKey) {
            const hasApiFields = profile.baseUrl.trim() && profile.apiKey.trim();
            if (!relay?.authenticated && !hasApiFields)
                return i18n.t("relayProfileReadiness.notLoggedInNoConfig");
            if (!relay?.authenticated)
                return i18n.t("relayProfileReadiness.notLoggedIn");
            if (!hasApiFields)
                return i18n.t("relayProfileReadiness.noMixConfig");
            return i18n.t("relayProfileReadiness.officialReady", { email: relay.accountLabel || i18n.t("relayProfileReadiness.loggedInFallback") });
        }
        return relay?.authenticated
            ? i18n.t("relayProfileReadiness.officialLoggedIn", { email: relay.accountLabel || relay.authSource || i18n.t("relayProfileReadiness.detectedFallback") })
            : i18n.t("relayProfileReadiness.notLoggedInSwitch");
    }
    const hasFiles = profile.configContents.trim() && profile.authContents.trim();
    if (!hasFiles)
        return i18n.t("relayProfileReadiness.noConfig");
    if (relay && !relay.configured)
        return i18n.t("relayProfileReadiness.pureApiIncomplete");
    return i18n.t("relayProfileReadiness.pureApiReady");
}
export function relayProfileSwitchCommand(profile: RelayProfile): "clear_relay_injection" | "apply_relay_injection" | "apply_pure_api_injection" {
    if (profile.relayMode === "pureApi")
        return "apply_pure_api_injection";
    if (profile.relayMode === "official" && !profile.officialMixApiKey)
        return "clear_relay_injection";
    if (profile.configContents.trim())
        return "apply_relay_injection";
    return profile.officialMixApiKey ? "apply_relay_injection" : "clear_relay_injection";
}
export function relayProfileModeSwitchedText(profile: RelayProfile): string {
    if (profile.relayMode === "pureApi")
        return i18n.t("relayProfileModeSwitched.pureApi");
    if (profile.officialMixApiKey)
        return i18n.t("relayProfileModeSwitched.mixOfficial");
    return i18n.t("relayProfileModeSwitched.official");
}
export function withGeneratedRelayFiles(profile: RelayProfile): RelayProfile {
    if (profile.relayMode === "official") {
        return {
            ...profile,
            configContents: profile.officialMixApiKey ? buildRelayConfigToml(profile, { includeBearerToken: true }) : "",
            authContents: profile.authContents || "",
        };
    }
    return {
        ...profile,
        configContents: buildRelayConfigToml(profile, { includeBearerToken: false }),
        authContents: buildRelayAuthJson(profile),
    };
}
export function buildRelayConfigToml(profile: Pick<RelayProfile, "model" | "baseUrl" | "upstreamBaseUrl" | "apiKey" | "protocol">, options: {
    includeBearerToken: boolean;
}): string {
    const baseUrl = profile.protocol === "chatCompletions" ? PROTOCOL_PROXY_BASE_URL : profile.baseUrl.trim();
    const apiKey = profile.apiKey.trim();
    const rootLines = [
        profile.model.trim() ? `model = "${tomlString(profile.model.trim())}"` : null,
        'model_provider = "custom"',
        "",
    ].filter((line): line is string => line !== null);
    return [
        ...rootLines,
        "[model_providers.custom]",
        'name = "custom"',
        'wire_api = "responses"',
        "requires_openai_auth = true",
        `base_url = "${tomlString(baseUrl)}"`,
        options.includeBearerToken && apiKey ? `experimental_bearer_token = "${tomlString(apiKey)}"` : null,
        "",
    ].filter((line): line is string => line !== null).join("\n");
}
export function buildRelayAuthJson(profile: Pick<RelayProfile, "apiKey">): string {
    return `${JSON.stringify({ OPENAI_API_KEY: profile.apiKey.trim() }, null, 2)}\n`;
}
export function buildOfficialRelayAuthJson(contents: string): string {
    const trimmed = contents.trim();
    if (!trimmed)
        return "";
    try {
        const parsed = JSON.parse(trimmed) as Record<string, unknown>;
        if (!parsed || typeof parsed !== "object" || Array.isArray(parsed))
            return "";
        delete parsed.OPENAI_API_KEY;
        return `${JSON.stringify(parsed, null, 2)}\n`;
    }
    catch {
        return "";
    }
}
export function deriveRelayProfileFromFiles(profile: RelayProfile): RelayProfile {
    const configContents = profile.configContents || "";
    const authContents = profile.relayMode === "official" ? buildOfficialRelayAuthJson(profile.authContents || "") : profile.authContents || "";
    const configBaseUrl = codexBaseUrlFromConfig(configContents);
    const chatUpstreamBaseUrl = rootTomlStringValue(configContents, CHAT_UPSTREAM_BASE_URL_KEY);
    const isProxyConfig = configBaseUrl === PROTOCOL_PROXY_BASE_URL;
    const upstreamBaseUrl = profile.upstreamBaseUrl || chatUpstreamBaseUrl || (configBaseUrl && !isProxyConfig ? configBaseUrl : profile.baseUrl || "");
    const configApiKey = codexExperimentalBearerTokenFromConfig(configContents);
    return {
        ...profile,
        model: codexModelFromConfig(configContents),
        baseUrl: upstreamBaseUrl,
        upstreamBaseUrl,
        apiKey: profile.relayMode === "official"
            ? configApiKey || profile.apiKey || ""
            : codexApiKeyFromAuth(authContents) || configApiKey || "",
        contextWindow: codexTopLevelIntFromConfig(configContents, "model_context_window"),
        autoCompactLimit: codexTopLevelIntFromConfig(configContents, "model_auto_compact_token_limit"),
        configContents,
        authContents,
    };
}
export function applyRelayProfilePatchToFiles(profile: RelayProfile, patch: Partial<RelayProfile>, options: {
    allowGenerateFiles?: boolean;
} = {}): RelayProfile {
    let next: RelayProfile = { ...profile, ...patch };
    const shouldHaveFiles = next.relayMode !== "official" || next.officialMixApiKey || next.configContents.trim() || next.authContents.trim();
    const needsAuthFile = next.relayMode === "pureApi";
    if (options.allowGenerateFiles && shouldHaveFiles && (!next.configContents.trim() || (needsAuthFile && !next.authContents.trim()))) {
        next = withGeneratedRelayFiles(next);
    }
    if ("model" in patch) {
        next.configContents = setRootTomlStringKey(next.configContents, "model", patch.model || "");
    }
    if ("apiKey" in patch) {
        if (next.relayMode === "pureApi") {
            next.authContents = setAuthOpenAiApiKey(next.authContents, patch.apiKey || "");
            next.configContents = removeCodexExperimentalBearerToken(next.configContents);
        }
        else {
            next.configContents = setCodexExperimentalBearerToken(next.configContents, patch.apiKey || "");
        }
    }
    if ("baseUrl" in patch) {
        next.upstreamBaseUrl = patch.baseUrl || "";
    }
    if ("upstreamBaseUrl" in patch) {
        next.baseUrl = patch.upstreamBaseUrl || "";
    }
    if ("baseUrl" in patch || "upstreamBaseUrl" in patch || "protocol" in patch) {
        const baseUrlForConfig = next.protocol === "chatCompletions" ? PROTOCOL_PROXY_BASE_URL : next.upstreamBaseUrl || next.baseUrl;
        next.configContents = setCodexProviderStringKey(next.configContents, "base_url", baseUrlForConfig);
        next.configContents = removeRootTomlKey(next.configContents, CHAT_UPSTREAM_BASE_URL_KEY);
    }
    if ("contextWindow" in patch) {
        next.configContents = setRootTomlIntKey(next.configContents, "model_context_window", patch.contextWindow || "");
    }
    if ("autoCompactLimit" in patch) {
        next.configContents = setRootTomlIntKey(next.configContents, "model_auto_compact_token_limit", patch.autoCompactLimit || "");
    }
    if ("relayMode" in patch || "officialMixApiKey" in patch) {
        if (next.relayMode === "official" && !next.officialMixApiKey) {
            next.configContents = "";
            next.authContents = buildOfficialRelayAuthJson(next.authContents);
        }
        else if (options.allowGenerateFiles && (!next.configContents.trim() || (next.relayMode === "pureApi" && !next.authContents.trim()))) {
            next = withGeneratedRelayFiles(next);
        }
    }
    return deriveRelayProfileFromFiles(next);
}
export function codexModelFromConfig(contents: string): string {
    for (const line of contents.split(/\r?\n/)) {
        const trimmed = line.trim();
        if (!trimmed || trimmed.startsWith("#"))
            continue;
        if (trimmed.startsWith("["))
            break;
        const match = /^model\s*=\s*(["'])(.*)\1\s*$/.exec(trimmed);
        if (match)
            return match[2].replace(/\\(["'\\])/g, "$1");
    }
    return "";
}
export function codexBaseUrlFromConfig(contents: string): string {
    return codexProviderStringFromConfig(contents, "base_url");
}
export function codexExperimentalBearerTokenFromConfig(contents: string): string {
    return codexProviderStringFromConfig(contents, "experimental_bearer_token");
}
export function codexProviderStringFromConfig(contents: string, key: string): string {
    const provider = rootTomlStringValue(contents, "model_provider");
    const targetSection = provider ? `model_providers.${provider}` : "";
    const lines = contents.split(/\r?\n/);
    let currentSection = "";
    const matches: string[] = [];
    for (const line of lines) {
        const section = tomlSectionName(line);
        if (section !== null) {
            currentSection = section;
            continue;
        }
        const value = tomlStringAssignmentValue(line, key);
        if (value === null)
            continue;
        if (targetSection && currentSection === targetSection)
            return value;
        if (!currentSection || !currentSection.startsWith("model_providers."))
            matches.push(value);
    }
    return matches.length === 1 ? matches[0] : "";
}
export function codexApiKeyFromAuth(contents: string): string {
    try {
        const parsed = JSON.parse(contents || "{}") as {
            OPENAI_API_KEY?: unknown;
        };
        return typeof parsed.OPENAI_API_KEY === "string" ? parsed.OPENAI_API_KEY : "";
    }
    catch {
        return "";
    }
}
export function codexTopLevelIntFromConfig(contents: string, key: string): string {
    const topLevel = splitTomlRootAndTables(contents).root;
    const pattern = new RegExp(`^\\s*${key}\\s*=\\s*(\\d+)\\s*(?:#.*)?$`);
    for (const line of topLevel.split(/\r?\n/)) {
        const match = pattern.exec(line);
        if (match)
            return match[1];
    }
    return "";
}
export function rootTomlStringValue(contents: string, key: string): string {
    const topLevel = splitTomlRootAndTables(contents).root;
    for (const line of topLevel.split(/\r?\n/)) {
        const value = tomlStringAssignmentValue(line, key);
        if (value !== null)
            return value;
    }
    return "";
}
export function tomlSectionName(line: string): string | null {
    const match = /^\s*\[([^\]]+)\]\s*$/.exec(line);
    return match ? match[1].trim() : null;
}
export function tomlStringAssignmentValue(line: string, key: string): string | null {
    const match = new RegExp(`^\\s*${key}\\s*=\\s*([\"'])(.*)\\1\\s*(?:#.*)?$`).exec(line.trim());
    if (!match)
        return null;
    return match[2].replace(/\\(["'\\])/g, "$1");
}
export function setAuthOpenAiApiKey(contents: string, apiKey: string): string {
    let parsed: Record<string, unknown> = {};
    try {
        const value = JSON.parse(contents || "{}");
        if (value && typeof value === "object" && !Array.isArray(value))
            parsed = value as Record<string, unknown>;
    }
    catch {
        parsed = {};
    }
    parsed.OPENAI_API_KEY = apiKey.trim();
    return `${JSON.stringify(parsed, null, 2)}\n`;
}
export function setRootTomlStringKey(contents: string, key: string, value: string): string {
    const trimmed = value.trim();
    if (!trimmed)
        return removeRootTomlKey(contents, key);
    return setRootTomlLine(contents, key, `${key} = "${tomlString(trimmed)}"`);
}
export function setRootTomlIntKey(contents: string, key: string, value: string): string {
    const trimmed = value.replace(/[^\d]/g, "");
    if (!trimmed)
        return removeRootTomlKey(contents, key);
    return setRootTomlLine(contents, key, `${key} = ${trimmed}`);
}
export function setRootTomlLine(contents: string, key: string, lineText: string): string {
    const lines = contents.split(/\r?\n/);
    const firstTable = lines.findIndex((line) => /^\s*\[[^\]]+\]\s*$/.test(line));
    const rootEnd = firstTable >= 0 ? firstTable : lines.length;
    for (let index = 0; index < rootEnd; index += 1) {
        if (new RegExp(`^\\s*${key}\\s*=`).test(lines[index])) {
            lines[index] = lineText;
            return ensureTrailingNewline(lines.join("\n").trimEnd());
        }
    }
    const insertAt = key === "model" ? 0 : rootEnd;
    lines.splice(insertAt, 0, lineText);
    return ensureTrailingNewline(lines.join("\n").trimEnd());
}
export function setCodexProviderStringKey(contents: string, key: string, value: string): string {
    const provider = rootTomlStringValue(contents, "model_provider") || "custom";
    let next = contents;
    if (!rootTomlStringValue(next, "model_provider")) {
        next = setRootTomlStringKey(next, "model_provider", provider);
    }
    next = ensureCodexProviderDefaults(next, provider);
    return setTomlSectionStringKey(next, `model_providers.${provider}`, key, value);
}
export function setCodexExperimentalBearerToken(contents: string, apiKey: string): string {
    const trimmed = apiKey.trim();
    return trimmed
        ? setCodexProviderStringKey(contents, "experimental_bearer_token", trimmed)
        : removeCodexExperimentalBearerToken(contents);
}
export function removeCodexExperimentalBearerToken(contents: string): string {
    const provider = rootTomlStringValue(contents, "model_provider") || "custom";
    return removeTomlSectionKey(contents, `model_providers.${provider}`, "experimental_bearer_token");
}
export function ensureCodexProviderDefaults(contents: string, provider: string): string {
    let next = contents;
    const section = `model_providers.${provider}`;
    next = setTomlSectionStringKey(next, section, "name", provider);
    next = setTomlSectionStringKey(next, section, "wire_api", "responses");
    return setTomlSectionBoolKey(next, section, "requires_openai_auth", true);
}
export function setTomlSectionBoolKey(contents: string, sectionName: string, key: string, value: boolean): string {
    return setTomlSectionRawKey(contents, sectionName, key, value ? "true" : "false");
}
export function setTomlSectionStringKey(contents: string, sectionName: string, key: string, value: string): string {
    return setTomlSectionRawKey(contents, sectionName, key, `"${tomlString(value.trim())}"`);
}
export function setTomlSectionRawKey(contents: string, sectionName: string, key: string, value: string): string {
    const lines = contents.split(/\r?\n/);
    let sectionStart = -1;
    let sectionEnd = lines.length;
    for (let index = 0; index < lines.length; index += 1) {
        const section = tomlSectionName(lines[index]);
        if (section === null)
            continue;
        if (sectionStart >= 0) {
            sectionEnd = index;
            break;
        }
        if (section === sectionName)
            sectionStart = index;
    }
    if (sectionStart < 0) {
        const prefix = ensureTrailingNewline(lines.join("\n").trimEnd()).trimEnd();
        return joinTomlSections([prefix, `[${sectionName}]\n${key} = ${value}`]);
    }
    const replacement = `${key} = ${value}`;
    for (let index = sectionStart + 1; index < sectionEnd; index += 1) {
        if (new RegExp(`^\\s*${key}\\s*=`).test(lines[index])) {
            lines[index] = replacement;
            return ensureTrailingNewline(lines.join("\n").trimEnd());
        }
    }
    let insertAt = sectionEnd;
    while (insertAt > sectionStart + 1 && lines[insertAt - 1].trim() === "")
        insertAt -= 1;
    lines.splice(insertAt, 0, replacement);
    return ensureTrailingNewline(lines.join("\n").trimEnd());
}
export function removeTomlSectionKey(contents: string, sectionName: string, key: string): string {
    const lines = contents.split(/\r?\n/);
    let sectionStart = -1;
    let sectionEnd = lines.length;
    for (let index = 0; index < lines.length; index += 1) {
        const section = tomlSectionName(lines[index]);
        if (section === null)
            continue;
        if (sectionStart >= 0) {
            sectionEnd = index;
            break;
        }
        if (section === sectionName)
            sectionStart = index;
    }
    if (sectionStart < 0)
        return contents;
    const next = lines.filter((line, index) => {
        if (index <= sectionStart || index >= sectionEnd)
            return true;
        return !new RegExp(`^\\s*${key}\\s*=`).test(line);
    });
    return ensureTrailingNewline(next.join("\n").trimEnd());
}
export function relayProfileSwitchValidation(profile: RelayProfile): string | null {
    if (profile.relayMode === "official" && !profile.officialMixApiKey)
        return null;
    if (!profile.configContents.trim()) {
        return i18n.t("relayProfileSwitchValidation.noConfig", { name: profile.name || profile.id });
    }
    if (profile.relayMode !== "official" || !authJsonHasOpenAiApiKey(profile.authContents))
        return null;
    return i18n.t("relayProfileSwitchValidation.mixAuthKey");
}
export function authJsonHasOpenAiApiKey(contents: string): boolean {
    const trimmed = contents.trim();
    if (!trimmed)
        return false;
    try {
        const value = JSON.parse(trimmed);
        return !!value && typeof value === "object" && typeof value.OPENAI_API_KEY === "string" && value.OPENAI_API_KEY.trim().length > 0;
    }
    catch {
        return /"OPENAI_API_KEY"\s*:/.test(trimmed);
    }
}
export function tomlString(value: string): string {
    return value.replace(/\\/g, "\\\\").replace(/"/g, '\\"');
}
export function syncLegacyRelayFields(settings: BackendSettings): BackendSettings {
    const relayProfiles = settings.relayProfiles.map(deriveRelayProfileFromFiles);
    const active = activeRelayProfile({ ...settings, relayProfiles });
    return {
        ...settings,
        relayProfiles,
        activeRelayId: active.id,
        relayBaseUrl: active.baseUrl,
        relayApiKey: active.apiKey,
    };
}
export function mergeLiveLinkedRelayProfiles(settings: BackendSettings, liveSettings: BackendSettings): BackendSettings {
    const liveLinkedById = new Map(liveSettings.relayProfiles
        .filter((profile) => profile.linkedCcsProviderId.trim())
        .map((profile) => [profile.id, profile]));
    if (!liveLinkedById.size)
        return settings;
    const existingIds = new Set(settings.relayProfiles.map((profile) => profile.id));
    const relayProfiles = [
        ...settings.relayProfiles.map((profile) => liveLinkedById.get(profile.id) ?? profile),
        ...liveSettings.relayProfiles.filter((profile) => profile.linkedCcsProviderId.trim() && !existingIds.has(profile.id)),
    ];
    return syncLegacyRelayFields({
        ...settings,
        relayProfiles,
        activeRelayId: relayProfiles.some((profile) => profile.id === settings.activeRelayId)
            ? settings.activeRelayId
            : liveSettings.activeRelayId,
    });
}
export function updateRelayProfile(settings: BackendSettings, id: string, patch: Partial<RelayProfile>): BackendSettings {
    return syncLegacyRelayFields({
        ...settings,
        relayProfiles: settings.relayProfiles.map((profile) => {
            if (profile.id !== id)
                return profile;
            return deriveRelayProfileFromFiles({ ...profile, ...patch });
        }),
    });
}
export function createRelayProfile(settings: BackendSettings): RelayProfile {
    const id = `relay-${Date.now().toString(36)}`;
    const contextSelection = contextSelectionForAllEntries(settings);
    const next = {
        id,
        linkedCcsProviderId: "",
        name: i18n.t("createRelayProfile.defaultName", { number: settings.relayProfiles.length + 1 }),
        model: "",
        baseUrl: defaultSettings.relayBaseUrl,
        upstreamBaseUrl: defaultSettings.relayBaseUrl,
        apiKey: "",
        protocol: "responses" as RelayProtocol,
        relayMode: "official" as RelayMode,
        officialMixApiKey: false,
        testModel: "",
        configContents: "",
        authContents: "",
        useCommonConfig: true,
        contextSelection,
        contextSelectionInitialized: true,
        contextWindow: "",
        autoCompactLimit: "",
        modelList: "",
        userAgent: "",
    };
    return withGeneratedRelayFiles(next);
}
export function addRelayProfile(settings: BackendSettings, profile: RelayProfile): BackendSettings {
    const nextWithFiles = deriveRelayProfileFromFiles(profile.configContents.trim() || profile.authContents.trim() ? profile : withGeneratedRelayFiles(profile));
    const activeId = settings.relayProfiles.some((item) => item.id === settings.activeRelayId)
        ? settings.activeRelayId
        : activeRelayProfile(settings).id;
    return syncLegacyRelayFields({
        ...settings,
        relayProfiles: [...settings.relayProfiles, nextWithFiles],
        activeRelayId: activeId,
    });
}
export function duplicateRelayProfile(settings: BackendSettings, id: string): BackendSettings {
    const sourceIndex = settings.relayProfiles.findIndex((profile) => profile.id === id);
    const source = settings.relayProfiles[sourceIndex] || activeRelayProfile(settings);
    const nextId = `relay-${Date.now().toString(36)}`;
    const next = {
        ...source,
        id: nextId,
        linkedCcsProviderId: "",
        name: i18n.t("duplicateRelayProfile.defaultName", { name: source.name || i18n.t("sortableCard.unnamed") }),
    };
    const relayProfiles = [...settings.relayProfiles];
    relayProfiles.splice(sourceIndex >= 0 ? sourceIndex + 1 : relayProfiles.length, 0, next);
    return syncLegacyRelayFields({
        ...settings,
        relayProfiles,
    });
}
export function reorderRelayProfiles(settings: BackendSettings, sourceId: string, targetId: string): BackendSettings {
    if (sourceId === targetId)
        return settings;
    const sourceIndex = settings.relayProfiles.findIndex((profile) => profile.id === sourceId);
    const targetIndex = settings.relayProfiles.findIndex((profile) => profile.id === targetId);
    if (sourceIndex < 0 || targetIndex < 0)
        return settings;
    const relayProfiles = [...settings.relayProfiles];
    const [moved] = relayProfiles.splice(sourceIndex, 1);
    relayProfiles.splice(targetIndex, 0, moved);
    return syncLegacyRelayFields({
        ...settings,
        relayProfiles,
    });
}
export function removeRelayProfile(settings: BackendSettings, id: string): BackendSettings {
    const profiles = settings.relayProfiles.filter((profile) => profile.id !== id);
    return syncLegacyRelayFields({
        ...settings,
        relayProfiles: profiles.length ? profiles : defaultSettings.relayProfiles,
        activeRelayId: settings.activeRelayId === id ? profiles[0]?.id || DEFAULT_RELAY_PROFILE_ID : settings.activeRelayId,
    });
}
export function numberOrDefault(value: string, fallback: number) {
    const parsed = Number.parseInt(value, 10);
    return Number.isFinite(parsed) ? parsed : fallback;
}
export function splitLogLines(text: string) {
    return text.trimEnd().split(/\r?\n/).filter((line, index, lines) => line.length > 0 || index < lines.length - 1);
}
export function formatTime(value: number) {
    if (!value)
        return "-";
    const locale = i18n.language === "zh" ? "zh-CN" : "en-US";
    return new Date(value).toLocaleString(locale);
}
export function stringifyError(error: unknown) {
    if (error instanceof Error)
        return error.message;
    return String(error);
}
export function loadInitialTheme(): Theme {
    if (typeof window === "undefined")
        return "dark";
    return window.localStorage.getItem(STORAGE_KEYS.theme) === "light" ? "light" : "dark";
}
export function loadInitialRoute(): Route {
    if (typeof window === "undefined")
        return "overview";
    const params = new URLSearchParams(window.location.search);
    if (params.get("showUpdate") === "1" || window.location.hash === "#about") {
        return "about";
    }
    return "overview";
}
