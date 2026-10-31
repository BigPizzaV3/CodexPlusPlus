/**
 * Agent 桌宠：在 Agent 窗口里放一个可拖动的小角色，点一下会被压扁再弹回来，并播放按压音效。
 *
 * 图片不随应用打包：内置图库只保存 id，预览和 Codex 里显示的图片都由后端运行时联网下载、
 * 缓存在本机；也可以填一个 https 图片地址。目前只有 Codex 能显示桌宠。
 * 音效同理：两组内嵌，其余音效组用到时才下载；也可以导入本地音频作为自定义按压 / 松开音。
 *
 * 桌宠类型二选一：上面这种「简单桌宠」，或完整的旧版「大肥鱼」（DeepSeek 鲸鱼用量挂件，
 * 含余额、价格 / 用量统计、泡泡编辑器、音效库与提醒）。两者互斥，同一时间只显示一个。
 * 大肥鱼的角色与素材随启动器内嵌（上游原样素材），它自己的配置在 Codex 里点角色旁的菜单完成；
 * 这里只放旧版增强页里的「API 余额」配置。
 */
import { useEffect, useRef, useState, type ReactNode } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Check, FileAudio, Fish, ImageOff, Image as ImageIcon, Link2, Loader2, PawPrint, RefreshCw, Volume2, X } from "lucide-react";
import { AppSelect } from "./AppSelect";
import { Button } from "./components/ui/button";
import { Input } from "./components/ui/input";
import { PageHeader } from "./ManagementPages";
import {
  AGENT_PET_CUSTOM_IMAGE,
  AGENT_PET_CUSTOM_SOUND,
  AGENT_PET_KIND_IMAGE,
  AGENT_PET_KIND_WHALE_WIDGET,
  AGENT_PET_EMBEDDED_SOUND_SETS,
  AGENT_PET_SIZE_MAX,
  AGENT_PET_SIZE_MIN,
  AGENT_PET_SUPPORTED_AGENTS,
  agentPetSettingsIssue,
  agentPetSoundKey,
  isAgentPetImageUrl,
  type AgentPetAgent,
  type AgentPetKind,
  type AgentPetSettings,
  type AgentPetSoundSlot,
} from "./agent-pet-settings";
import { AgentPetSoundPlayer, type AgentPetSoundSources } from "./agent-pet-sound";
import type { WhaleBalanceProtocol, WhaleBalanceSettings } from "./whale-settings";
import { t, tf } from "./i18n";
import "./AgentPetPage.css";

type CatalogEntry = { id: string; name: string; credit?: string };
type ImageResult = { status: string; image?: string; message?: string };
type PreviewState = { state: "loading" } | { state: "ok"; image: string } | { state: "failed"; message: string };
type SoundEntry = { id: string; name: string; author: string; license: string; sourceUrl: string };
type SoundResult = { status: string; key?: string; press?: string; release?: string; message?: string };
type ImportResult = { status: string; file?: string; message?: string };
type SoundState = { state: "loading" } | { state: "ok" } | { state: "failed"; message: string };

const agentOptions: Array<{ id: AgentPetAgent; label: string }> = [
  { id: "codex", label: "Codex" },
  { id: "claude", label: "Claude Code" },
  { id: "grok", label: "Grok" },
];
/** 图库预览同时下载的张数：首次打开时不一下子发十几个请求。 */
const PREVIEW_CONCURRENCY = 4;

const errorText = (error: unknown) => (error instanceof Error ? error.message : String(error));
/** 与后端 `MAX_SOUND_BYTES` 一致，超过的文件在前端直接拦下，不必传过去。 */
const SOUND_FILE_MAX_BYTES = 2 * 1024 * 1024;
const embeddedSoundOptions: Array<[string, string]> = [["duck", "小黄鸭"], ["fx1", "音效 1"]];

function bytesToBase64(buffer: ArrayBuffer): string {
  const bytes = new Uint8Array(buffer);
  let binary = "";
  // 分块拼接，避免大文件一次展开参数撑爆调用栈。
  for (let index = 0; index < bytes.length; index += 0x8000) {
    binary += String.fromCharCode(...bytes.subarray(index, index + 0x8000));
  }
  return btoa(binary);
}

async function loadImage(image: string, customImageUrl = ""): Promise<PreviewState> {
  try {
    const result = await invoke<ImageResult>("agent_pet_image", { image, customImageUrl });
    if (result.status === "ok" && result.image) return { state: "ok", image: result.image };
    return { state: "failed", message: result.message || t("图片下载失败") };
  } catch (error) {
    return { state: "failed", message: errorText(error) };
  }
}

export function AgentPetPage({ settings, onChange, autoSave, whaleBalance, onWhaleBalanceChange, whaleBalanceIssue = null }: {
  settings: AgentPetSettings;
  onChange: (next: AgentPetSettings) => void;
  /** 自动保存状态，由主界面提供（与其他设置页同一套保存逻辑）。 */
  autoSave: ReactNode;
  /** 大肥鱼的余额配置（设置里顶层的 `codexAppWhaleBalance*`）。 */
  whaleBalance: WhaleBalanceSettings;
  onWhaleBalanceChange: (patch: Partial<WhaleBalanceSettings>) => void;
  /** 余额配置的校验提示，由主界面按保存时同一规则算出。 */
  whaleBalanceIssue?: string | null;
}) {
  const update = (patch: Partial<AgentPetSettings>) => onChange({ ...settings, ...patch });
  const isWhale = settings.kind === AGENT_PET_KIND_WHALE_WIDGET;
  // 大肥鱼的角色图随应用内嵌，由后端交给页面预览。
  const [whaleImage, setWhaleImage] = useState<PreviewState | null>(null);
  useEffect(() => {
    if (!isWhale || whaleImage) return;
    setWhaleImage({ state: "loading" });
    void invoke<string>("agent_pet_whale_image")
      .then((image) => setWhaleImage(image ? { state: "ok", image } : { state: "failed", message: t("大肥鱼预览暂不可用") }))
      .catch((error) => setWhaleImage({ state: "failed", message: errorText(error) }));
  }, [isWhale, whaleImage]);
  const selectKind = (kind: AgentPetKind) => update({ kind });
  const [catalog, setCatalog] = useState<CatalogEntry[] | null>(null);
  const [catalogError, setCatalogError] = useState("");
  const [previews, setPreviews] = useState<Record<string, PreviewState>>({});
  const [customDraft, setCustomDraft] = useState(settings.customImageUrl);
  const [customPreview, setCustomPreview] = useState<PreviewState | null>(null);
  // 试听用的音效与 Codex 里是同一份内嵌音频，由后端交给页面。
  const playerRef = useRef<AgentPetSoundPlayer | null>(null);
  if (!playerRef.current) playerRef.current = new AgentPetSoundPlayer({});
  const player = playerRef.current;
  useEffect(() => {
    let alive = true;
    void invoke<AgentPetSoundSources>("agent_pet_sounds").then((sources) => {
      if (alive) player.setSources(sources);
    }).catch(() => undefined);
    return () => { alive = false; player.dispose(); };
  }, [player]);
  const soundOn = settings.soundEnabled;
  const soundKey = agentPetSoundKey(settings);
  const pressSound = () => { if (soundOn) void player.pressDown(soundKey, settings.volume); };
  const releaseSound = () => { if (soundOn) void player.pressUp(soundKey, settings.volume); };

  // 可下载音效目录与各组的下载状态；内嵌组不需要下载。
  const [soundCatalog, setSoundCatalog] = useState<SoundEntry[]>([]);
  const [soundStates, setSoundStates] = useState<Record<string, SoundState>>({});
  const [importing, setImporting] = useState<AgentPetSoundSlot | null>(null);
  const [importError, setImportError] = useState("");
  const fileInputs = { press: useRef<HTMLInputElement>(null), release: useRef<HTMLInputElement>(null) };
  useEffect(() => {
    void invoke<SoundEntry[]>("agent_pet_sound_catalog").then(setSoundCatalog).catch(() => undefined);
  }, []);

  /** 取一组下载 / 自定义音效交给试听播放器；返回是否可用。 */
  const loadSound = async (next: Pick<AgentPetSettings, "soundSet" | "customSoundPress" | "customSoundRelease">) => {
    const key = agentPetSoundKey(next);
    if (AGENT_PET_EMBEDDED_SOUND_SETS.includes(next.soundSet) || player.has(key)) return true;
    setSoundStates((current) => ({ ...current, [next.soundSet]: { state: "loading" } }));
    try {
      const result = await invoke<SoundResult>("agent_pet_sound", {
        soundSet: next.soundSet,
        customSoundPress: next.customSoundPress,
        customSoundRelease: next.customSoundRelease,
      });
      if (result.status !== "ok" || !result.key) throw new Error(result.message || t("音效下载失败"));
      player.setSource(result.key, { press: result.press, release: result.release });
      setSoundStates((current) => ({ ...current, [next.soundSet]: { state: "ok" } }));
      return true;
    } catch (error) {
      setSoundStates((current) => ({ ...current, [next.soundSet]: { state: "failed", message: errorText(error) } }));
      return false;
    }
  };
  // 已保存的是下载组或自定义组：进入页面时取一次，预览按压也有声音。
  useEffect(() => {
    void loadSound(settings);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // 下载组只有下载成功才切过去，避免 Codex 里的桌宠没声音。
  const selectSound = async (soundSet: string) => {
    const next = { ...settings, soundSet };
    if (!(await loadSound(next))) return;
    update({ soundSet });
    void player.preview(agentPetSoundKey(next), settings.volume);
  };

  const importSound = async (slot: AgentPetSoundSlot, file: File | undefined) => {
    if (!file) return;
    setImportError("");
    if (file.size > SOUND_FILE_MAX_BYTES) {
      setImportError(t("音频文件超过 2 MB，请换一个短一点的文件。"));
      return;
    }
    setImporting(slot);
    try {
      const data = bytesToBase64(await file.arrayBuffer());
      const result = await invoke<ImportResult>("agent_pet_import_sound", { slot, data });
      if (result.status !== "ok" || !result.file) throw new Error(result.message || t("导入失败"));
      const next = {
        ...settings,
        soundSet: AGENT_PET_CUSTOM_SOUND,
        [slot === "press" ? "customSoundPress" : "customSoundRelease"]: result.file,
      };
      if (await loadSound(next)) {
        update(next);
        void player.preview(agentPetSoundKey(next), settings.volume);
      }
    } catch (error) {
      setImportError(errorText(error));
    } finally {
      setImporting(null);
    }
  };

  const clearCustomSound = (slot: AgentPetSoundSlot) => {
    const next = { ...settings, [slot === "press" ? "customSoundPress" : "customSoundRelease"]: "" };
    // 两个都清空了就回到默认音效组。
    if (!next.customSoundPress && !next.customSoundRelease && next.soundSet === AGENT_PET_CUSTOM_SOUND) next.soundSet = "duck";
    update(next);
    void loadSound(next);
  };
  const hasCustomSound = Boolean(settings.customSoundPress || settings.customSoundRelease);
  const selectedSoundEntry = soundCatalog.find((entry) => entry.id === settings.soundSet);
  const soundState = soundStates[settings.soundSet];

  const loadCatalog = async () => {
    setCatalogError("");
    try {
      setCatalog(await invoke<CatalogEntry[]>("agent_pet_catalog"));
    } catch (error) {
      setCatalogError(errorText(error));
    }
  };
  useEffect(() => { void loadCatalog(); }, []);

  // 按顺序下载图库预览，限制并发；失败的条目保留错误，点「重试」再拉。
  const previewsRef = useRef(previews);
  previewsRef.current = previews;
  const fetchPreviews = async (ids: string[]) => {
    const queue = [...ids];
    setPreviews((current) => ({ ...current, ...Object.fromEntries(ids.map((id) => [id, { state: "loading" } as PreviewState])) }));
    const worker = async () => {
      for (let id = queue.shift(); id; id = queue.shift()) {
        const result = await loadImage(id);
        setPreviews((current) => ({ ...current, [id]: result }));
      }
    };
    await Promise.all(Array.from({ length: PREVIEW_CONCURRENCY }, worker));
  };
  useEffect(() => {
    if (!catalog) return;
    void fetchPreviews(catalog.map((entry) => entry.id).filter((id) => !previewsRef.current[id]));
    // 只在图库加载完成时拉一次。
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [catalog]);

  // 已保存的自定义图片：进入页面时预览一次。
  useEffect(() => {
    if (settings.image === AGENT_PET_CUSTOM_IMAGE && isAgentPetImageUrl(settings.customImageUrl)) {
      setCustomPreview({ state: "loading" });
      void loadImage(AGENT_PET_CUSTOM_IMAGE, settings.customImageUrl).then(setCustomPreview);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const applyCustomUrl = async () => {
    const url = customDraft.trim();
    if (!isAgentPetImageUrl(url)) {
      setCustomPreview({ state: "failed", message: t("请填写 https:// 开头的图片地址") });
      return;
    }
    setCustomPreview({ state: "loading" });
    const result = await loadImage(AGENT_PET_CUSTOM_IMAGE, url);
    setCustomPreview(result);
    // 只有图片能下载下来才切过去，避免 Codex 里的桌宠变成空白。
    if (result.state === "ok") update({ image: AGENT_PET_CUSTOM_IMAGE, customImageUrl: url });
  };

  const toggleAgent = (agent: AgentPetAgent, checked: boolean) => {
    const agents = checked
      ? Array.from(new Set([...settings.agents, agent]))
      : settings.agents.filter((item) => item !== agent);
    update({ agents });
  };

  const selectedPreview = settings.image === AGENT_PET_CUSTOM_IMAGE ? customPreview : previews[settings.image];
  const issue = agentPetSettingsIssue(settings);
  const failedIds = Object.entries(previews).filter(([, value]) => value.state === "failed").map(([id]) => id);

  return (
    <div className="settings-page agent-pet-page">
      <PageHeader icon={<PawPrint className="h-4 w-4" />} title={t("Agent 桌宠")} />
      <div className="agent-pet-toolbar">
        <label className="feature-toggle agent-pet-master">
          <input
            checked={settings.enabled}
            onChange={(event) => update({ enabled: event.currentTarget.checked })}
            type="checkbox"
          />
          <span>
            <strong>{t("显示 Agent 桌宠")}</strong>
            <small>{t("在 Agent 窗口里放一个可拖动的小角色，点一下会被压扁再弹回来。改动几秒内同步到 Codex，不用重启。")}</small>
          </span>
          <span aria-hidden="true" className="toggle-switch-visual"><span className="toggle-switch-thumb" /></span>
        </label>
        {autoSave}
      </div>

      <div className={`agent-pet-layout ${settings.enabled ? "" : "is-off"}`}>
        <div className="agent-pet-main">
          <section className="feature-group">
            <div className="feature-group-head">
              <strong>{t("显示在")}</strong>
              <small>{t("目前只支持 Codex，其他 Agent 后续接入。")}</small>
            </div>
            <div className="agent-pet-agents" role="group" aria-label={t("显示在")}>
              {agentOptions.map((option) => {
                const supported = AGENT_PET_SUPPORTED_AGENTS.includes(option.id);
                const checked = supported && settings.agents.includes(option.id);
                return (
                  <label key={option.id} className={`agent-pet-agent ${checked ? "is-checked" : ""} ${supported ? "" : "is-disabled"}`}>
                    <input
                      type="checkbox"
                      checked={checked}
                      disabled={!supported}
                      onChange={(event) => toggleAgent(option.id, event.currentTarget.checked)}
                    />
                    <span>{option.label}</span>
                    {supported ? (checked ? <Check className="h-3.5 w-3.5" /> : null) : <small>{t("即将支持")}</small>}
                  </label>
                );
              })}
            </div>
          </section>

          <section className="feature-group">
            <div className="feature-group-head">
              <strong>{t("桌宠类型")}</strong>
              <small>{t("二选一，同一时间只显示一个。")}</small>
            </div>
            <div className="agent-pet-kinds" role="radiogroup" aria-label={t("桌宠类型")}>
              {([
                [AGENT_PET_KIND_IMAGE, <ImageIcon key="icon" className="h-5 w-5" />, t("简单桌宠"), t("从图库挑一张图或用自己的图片，点一下压扁回弹，带按压音效。")],
                [AGENT_PET_KIND_WHALE_WIDGET, <Fish key="icon" className="h-5 w-5" />, t("大肥鱼（DeepSeek 鲸鱼）"), t("原版鲸鱼用量挂件：余额、价格与用量统计、泡泡编辑器、音效库和提醒。")],
              ] as Array<[AgentPetKind, ReactNode, string, string]>).map(([kind, icon, title, detail]) => (
                <button
                  key={kind}
                  type="button"
                  role="radio"
                  aria-checked={settings.kind === kind}
                  className={`agent-pet-kind ${settings.kind === kind ? "is-selected" : ""}`}
                  onClick={() => selectKind(kind)}
                >
                  <span className="agent-pet-kind-icon">{icon}</span>
                  <span className="agent-pet-kind-text">
                    <strong>{title}</strong>
                    <small>{detail}</small>
                  </span>
                  {settings.kind === kind ? <Check className="h-4 w-4" /> : null}
                </button>
              ))}
            </div>
          </section>

          {isWhale ? (
            <WhaleWidgetSection balance={whaleBalance} onChange={onWhaleBalanceChange} issue={whaleBalanceIssue} />
          ) : <>
          <section className="feature-group">
            <div className="feature-group-head agent-pet-group-head">
              <div>
                <strong>{t("形象")}</strong>
                <small>{t("图片在用到时联网下载并缓存在本机，不随 Codex++ 打包。内置图库来自 Microsoft Fluent Emoji（MIT），二次元形象来自 Wikimedia Commons（CC0）。")}</small>
              </div>
              {failedIds.length ? (
                <Button size="sm" variant="secondary" onClick={() => void fetchPreviews(failedIds)}>
                  <RefreshCw className="h-4 w-4" />
                  {t("重试下载")}
                </Button>
              ) : null}
            </div>
            {catalogError ? <p className="agent-pet-error" role="alert">{catalogError}</p> : null}
            {!catalog && !catalogError ? <p className="agent-pet-muted"><Loader2 className="h-4 w-4 animate-spin" /> {t("正在加载图库…")}</p> : null}
            {catalog ? (
              <div className="agent-pet-gallery" role="radiogroup" aria-label={t("形象")}>
                {catalog.map((entry) => {
                  const preview = previews[entry.id];
                  const selected = settings.image === entry.id;
                  return (
                    <button
                      key={entry.id}
                      type="button"
                      role="radio"
                      aria-checked={selected}
                      className={`agent-pet-tile ${selected ? "is-selected" : ""}`}
                      title={preview?.state === "failed" ? preview.message : entry.credit ? `${entry.name} · ${entry.credit}` : entry.name}
                      onClick={() => update({ image: entry.id })}
                    >
                      <span className="agent-pet-tile-image">
                        {preview?.state === "ok" ? <img src={preview.image} alt="" draggable={false} /> : null}
                        {!preview || preview.state === "loading" ? <Loader2 className="h-5 w-5 animate-spin" /> : null}
                        {preview?.state === "failed" ? <ImageOff className="h-5 w-5" /> : null}
                      </span>
                      <span className="agent-pet-tile-name">{entry.name}</span>
                    </button>
                  );
                })}
              </div>
            ) : null}
            <div className={`agent-pet-custom ${settings.image === AGENT_PET_CUSTOM_IMAGE ? "is-selected" : ""}`}>
              <Link2 className="h-4 w-4" />
              <Input
                aria-label={t("自定义图片地址")}
                placeholder={t("自定义图片：https://…/pet.png（PNG / JPEG / WebP / GIF，4 MB 以内）")}
                value={customDraft}
                onChange={(event) => { const value = event.currentTarget.value; setCustomDraft(value); }}
                onKeyDown={(event) => { if (event.key === "Enter") void applyCustomUrl(); }}
              />
              <Button size="sm" variant="secondary" disabled={customPreview?.state === "loading"} onClick={() => void applyCustomUrl()}>
                {customPreview?.state === "loading" ? <Loader2 className="h-4 w-4 animate-spin" /> : null}
                {t("使用这张")}
              </Button>
            </div>
            {customPreview?.state === "failed" ? <p className="agent-pet-error" role="alert">{customPreview.message}</p> : null}
            {issue === "customUrl" ? <p className="agent-pet-error" role="alert">{t("自定义图片地址无效，请填写 https:// 开头的图片地址。")}</p> : null}
          </section>

          <section className="feature-group">
            <div className="feature-group-head">
              <strong>{t("大小与音效")}</strong>
              <small>{t("按下时播放按压音，松开后接上回弹音。")}</small>
            </div>
            <div className="agent-pet-controls">
              <label className="agent-pet-range">
                <span>{t("大小")}</span>
                <input
                  type="range"
                  min={AGENT_PET_SIZE_MIN}
                  max={AGENT_PET_SIZE_MAX}
                  step={4}
                  value={settings.size}
                  onChange={(event) => { const size = event.currentTarget.valueAsNumber; update({ size }); }}
                />
                <output>{settings.size}px</output>
              </label>
              <label className="feature-toggle">
                <input
                  checked={settings.soundEnabled}
                  onChange={(event) => update({ soundEnabled: event.currentTarget.checked })}
                  type="checkbox"
                />
                <span>
                  <strong>{t("按压音效")}</strong>
                  <small>{t("关闭后只保留压扁回弹的动画。")}</small>
                </span>
                <span aria-hidden="true" className="toggle-switch-visual"><span className="toggle-switch-thumb" /></span>
              </label>
              <div className={`agent-pet-sound ${settings.soundEnabled ? "" : "is-off"}`}>
                <div className="agent-pet-sound-row">
                <div className="agent-pet-segmented" role="radiogroup" aria-label={t("音效")}>
                  {[
                    ...embeddedSoundOptions.map(([value, label]) => [value, t(label)] as [string, string]),
                    ...soundCatalog.map((entry) => [entry.id, entry.name] as [string, string]),
                  ].map(([value, label]) => {
                    const state = soundStates[value];
                    return (
                      <button
                        key={value}
                        type="button"
                        role="radio"
                        aria-checked={settings.soundSet === value}
                        className={settings.soundSet === value ? "is-selected" : ""}
                        disabled={!settings.soundEnabled || state?.state === "loading"}
                        title={state?.state === "failed" ? state.message : undefined}
                        onClick={() => void selectSound(value)}
                      >
                        {state?.state === "loading" ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : null}
                        {label}
                      </button>
                    );
                  })}
                  <button
                    type="button"
                    role="radio"
                    aria-checked={settings.soundSet === AGENT_PET_CUSTOM_SOUND}
                    className={settings.soundSet === AGENT_PET_CUSTOM_SOUND ? "is-selected" : ""}
                    disabled={!settings.soundEnabled || !hasCustomSound}
                    title={hasCustomSound ? undefined : t("先在下面导入音频文件")}
                    onClick={() => void selectSound(AGENT_PET_CUSTOM_SOUND)}
                  >
                    {t("自定义")}
                  </button>
                </div>
                <Button
                  size="sm"
                  variant="secondary"
                  disabled={!settings.soundEnabled || settings.volume <= 0}
                  onClick={() => void player.preview(soundKey, settings.volume)}
                >
                  <Volume2 className="h-4 w-4" />
                  {t("试听")}
                </Button>
                </div>
                {selectedSoundEntry ? (
                  <small className="agent-pet-muted agent-pet-credit" title={selectedSoundEntry.sourceUrl}>
                    {tf("用到时下载并缓存在本机。来源：{0}（{1}）", [selectedSoundEntry.author, selectedSoundEntry.license])}
                  </small>
                ) : null}
                {soundState?.state === "failed" ? <p className="agent-pet-error" role="alert">{soundState.message}</p> : null}
                <div className="agent-pet-custom-sounds" aria-label={t("自定义音效")}>
                  {([["press", t("按压音"), settings.customSoundPress], ["release", t("松开音"), settings.customSoundRelease]] as Array<[AgentPetSoundSlot, string, string]>).map(([slot, label, file]) => (
                    <div key={slot} className="agent-pet-custom-sound">
                      <FileAudio className="h-4 w-4" />
                      <span className="agent-pet-custom-sound-label">{label}</span>
                      <span className="agent-pet-muted agent-pet-custom-sound-file">{file ? t("已导入") : t("未设置")}</span>
                      <input
                        ref={fileInputs[slot]}
                        type="file"
                        accept="audio/mpeg,audio/ogg,audio/wav,audio/mp4,.mp3,.ogg,.wav,.m4a"
                        hidden
                        onChange={(event) => { const input = event.currentTarget; void importSound(slot, input.files?.[0]).finally(() => { input.value = ""; }); }}
                      />
                      <Button size="sm" variant="secondary" disabled={!settings.soundEnabled || importing !== null} onClick={() => fileInputs[slot].current?.click()}>
                        {importing === slot ? <Loader2 className="h-4 w-4 animate-spin" /> : null}
                        {file ? t("更换") : t("选择文件")}
                      </Button>
                      {file ? (
                        <Button size="sm" variant="ghost" aria-label={tf("清除{0}", [label])} disabled={!settings.soundEnabled} onClick={() => clearCustomSound(slot)}>
                          <X className="h-4 w-4" />
                        </Button>
                      ) : null}
                    </div>
                  ))}
                  <small className="agent-pet-muted">{t("导入本地 MP3 / OGG / WAV / M4A（2 MB 以内），复制到 Codex++ 状态目录，可只设其中一个。")}</small>
                  {importError ? <p className="agent-pet-error" role="alert">{importError}</p> : null}
                </div>
                <label className="agent-pet-range">
                  <span>{t("音量")}</span>
                  <input
                    type="range"
                    min={0}
                    max={100}
                    step={5}
                    disabled={!settings.soundEnabled}
                    value={Math.round(settings.volume * 100)}
                    onChange={(event) => { const volume = event.currentTarget.valueAsNumber / 100; update({ volume }); }}
                  />
                  <output>{Math.round(settings.volume * 100)}%</output>
                </label>
              </div>
            </div>
          </section>
          </>}
        </div>

        {isWhale ? (
          <aside className="agent-pet-preview" aria-label={t("预览")}>
            <span className="agent-pet-preview-label">{t("预览")}</span>
            <div className="agent-pet-stage">
              {whaleImage?.state === "ok" ? (
                <img className="agent-pet-whale-figure" src={whaleImage.image} alt={t("大肥鱼（DeepSeek 鲸鱼）")} draggable={false} />
              ) : whaleImage?.state === "failed" ? (
                <span className="agent-pet-muted"><ImageOff className="h-5 w-5" /> {whaleImage.message}</span>
              ) : (
                <Loader2 className="h-6 w-6 animate-spin" />
              )}
            </div>
            <small className="agent-pet-muted">{t("在 Codex 里可以拖动、缩放它，点角色旁的菜单配置泡泡、音效、提醒和用量统计。")}</small>
          </aside>
        ) : (
        <aside className="agent-pet-preview" aria-label={t("预览")}>
          <span className="agent-pet-preview-label">{t("预览 · 按住试试")}</span>
          <div className="agent-pet-stage">
            {selectedPreview?.state === "ok" ? (
              <PetPreview image={selectedPreview.image} size={settings.size} onPress={pressSound} onRelease={releaseSound} />
            ) : selectedPreview?.state === "failed" ? (
              <span className="agent-pet-muted"><ImageOff className="h-5 w-5" /> {selectedPreview.message}</span>
            ) : (
              <Loader2 className="h-6 w-6 animate-spin" />
            )}
          </div>
          <small className="agent-pet-muted">{t("在 Codex 里可以拖动它换位置，位置会记住。")}</small>
        </aside>
        )}
      </div>
    </div>
  );
}

/**
 * 大肥鱼的设置：旧版增强页「Codex 用量挂件 → API 余额（可选）」原样搬到这里。
 * 余额查询复用 Codex++ 当前供应商的密钥，只在本机后端使用。
 */
function WhaleWidgetSection({ balance, onChange, issue }: {
  balance: WhaleBalanceSettings;
  onChange: (patch: Partial<WhaleBalanceSettings>) => void;
  issue: string | null;
}) {
  return (
    <section className="feature-group agent-pet-whale">
      <div className="feature-group-head">
        <strong>{t("API 余额（可选）")}</strong>
        <small>{t("跟随 Codex++ 当前供应商查询余额；Codex 订阅额度和本地 token 无需配置此项。")}</small>
      </div>
      <div className="agent-pet-whale-fields">
        <label className="agent-pet-whale-field">
          <span>{t("余额查询方式")}</span>
          <AppSelect<WhaleBalanceProtocol>
            value={balance.codexAppWhaleBalanceProtocol}
            ariaLabel={t("余额查询方式")}
            onChange={(value) => onChange({ codexAppWhaleBalanceProtocol: value })}
            options={[
              { value: "auto", label: t("自动识别已支持的接口") },
              { value: "off", label: t("仅显示 Codex 用量") },
              { value: "custom", label: t("自定义供应商余额接口") },
            ]}
          />
        </label>
        {balance.codexAppWhaleBalanceProtocol === "custom" ? <>
          <small className="agent-pet-muted">{t("按供应商文档填写 GET 接口和返回字段，复用当前供应商的 API Key。币种和倍率应与供应商账单一致。")}</small>
          <label className="agent-pet-whale-field">
            <span>{t("余额接口路径")}</span>
            <Input value={balance.codexAppWhaleBalancePath} placeholder="/api/balance" onChange={(event) => onChange({ codexAppWhaleBalancePath: event.currentTarget.value })} />
          </label>
          <label className="agent-pet-whale-field">
            <span>{t("余额字段路径")}</span>
            <Input value={balance.codexAppWhaleBalanceField} placeholder="data.balance" onChange={(event) => onChange({ codexAppWhaleBalanceField: event.currentTarget.value })} />
          </label>
          <div className="agent-pet-whale-row">
            <label className="agent-pet-whale-field">
              <span>{t("余额币种")}</span>
              <Input maxLength={3} value={balance.codexAppWhaleBalanceCurrency} placeholder="USD" onChange={(event) => onChange({ codexAppWhaleBalanceCurrency: event.currentTarget.value.toUpperCase() })} />
            </label>
            <label className="agent-pet-whale-field">
              <span>{t("金额倍率")}</span>
              <Input type="number" step="any" min={0} value={Number.isFinite(balance.codexAppWhaleBalanceScale) ? balance.codexAppWhaleBalanceScale : ""} onChange={(event) => onChange({ codexAppWhaleBalanceScale: event.currentTarget.valueAsNumber })} />
            </label>
          </div>
          <small className="agent-pet-muted">{t("显示金额 = 接口数值 × 倍率；例如接口返回分时，倍率填 0.01。")}</small>
          {issue ? <p className="agent-pet-error" role="alert">{issue}</p> : null}
        </> : null}
      </div>
      <small className="agent-pet-muted agent-pet-credit">
        {t("鲸鱼角色、完整界面引擎及内置音效 / 动图来自 MeteorNOX/DeepSeek-Balance-Whale-Widget（代码 MIT；素材按上游声明原样提供，不重新授权，也不声明为本项目原创）。")}
      </small>
    </section>
  );
}

/** 预览里的压扁回弹与 Codex 里一致：以底边为支点，按下压扁，松开弹回。 */
function PetPreview({ image, size, onPress, onRelease }: { image: string; size: number; onPress: () => void; onRelease: () => void }) {
  const [pressed, setPressed] = useState(false);
  // 只在真正按着时触发松开音：pointerup 之后还会收到 lostpointercapture，不能重复播放。
  const pressedRef = useRef(false);
  const release = () => {
    if (!pressedRef.current) return;
    pressedRef.current = false;
    setPressed(false);
    onRelease();
  };
  return (
    <button
      type="button"
      className={`agent-pet-figure ${pressed ? "is-pressed" : ""}`}
      style={{ width: size, height: size }}
      aria-label={t("按住预览压扁效果")}
      onPointerDown={(event) => { event.currentTarget.setPointerCapture(event.pointerId); pressedRef.current = true; setPressed(true); onPress(); }}
      onPointerUp={release}
      onPointerCancel={release}
      onLostPointerCapture={release}
    >
      <img src={image} alt="" draggable={false} />
    </button>
  );
}
