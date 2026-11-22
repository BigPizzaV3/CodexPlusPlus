import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { AppSelect } from "./AppSelect";
import { CxBanner, CxCard, CxRow, CxSection } from "./cx-layout";
import { Switch } from "./gateway-ui";
import { appConfirm } from "./app-dialog";
import { t } from "./i18n";

type Provider = "dingtalk" | "feishu";
type Network = "lan" | "tailscale" | "none";
type Status = { enabled: boolean; hasWebhook: boolean; hasSecret: boolean; linkNetwork: Network; lastResult: string | null };
type RobotStatus = Record<Provider, Status>;

const labels = { dingtalk: () => t("钉钉"), feishu: () => t("飞书") };
const request = (action: "status" | "save" | "test" | "clear", provider?: Provider, update?: Record<string, unknown>) =>
  invoke<RobotStatus>("chat_robot_request", { action, provider, update });

function RobotForm({ provider, status, busy, run }: {
  provider: Provider;
  status: Status;
  busy: boolean;
  run: (action: "save" | "test" | "clear", provider: Provider, update?: Record<string, unknown>) => Promise<boolean>;
}) {
  const [editing, setEditing] = useState(false);
  const [webhook, setWebhook] = useState("");
  const [secret, setSecret] = useState("");
  const [clearSecret, setClearSecret] = useState(false);
  const [network, setNetwork] = useState<Network>(status.linkNetwork);
  const name = labels[provider]();

  const toggleEditing = () => {
    setWebhook("");
    setSecret("");
    setClearSecret(false);
    setNetwork(status.linkNetwork);
    setEditing(!editing);
  };
  const save = async () => {
    const ok = await run("save", provider, {
      ...(webhook.trim() ? { webhook: webhook.trim() } : {}),
      ...(secret.trim() ? { secret: secret.trim() } : {}),
      clearSecret, linkNetwork: network,
    });
    if (ok) toggleEditing();
  };
  const clear = async () => {
    if (await appConfirm(t("清除后将停止向这个机器人推送。"), { title: t("清除机器人配置"), danger: true, confirmText: t("清除") })) {
      if (await run("clear", provider)) toggleEditing();
    }
  };

  return <CxCard>
    <CxRow title={name} detail={status.lastResult || (status.hasWebhook ? t("已保存机器人") : t("尚未配置机器人"))}>
      <button className="mg-button" disabled={busy} type="button" onClick={toggleEditing}>{editing ? t("取消") : t("配置机器人")}</button>
      {status.hasWebhook && <button className="mg-button" disabled={busy || editing} type="button" onClick={() => void run("test", provider)}>{t("测试通知")}</button>}
      <Switch label={name} on={status.enabled} pending={busy} disabled={busy || !status.hasWebhook || editing}
        onToggle={() => void run("save", provider, { enabled: !status.enabled })} />
    </CxRow>
    {editing && <>
      <CxRow title={t("机器人 Webhook")} detail={t("在群里添加自定义机器人，复制它的 Webhook 地址。如果启用关键词校验，请添加 Codex++。") }>
        <input aria-label={`${name} Webhook`} type="password" autoComplete="off" spellCheck={false} disabled={busy}
          placeholder={status.hasWebhook ? t("已保存，留空保留") : "https://…"} value={webhook} onChange={event => setWebhook(event.target.value)} />
      </CxRow>
      <CxRow title={t("机器人签名密钥")} detail={t("使用加签的机器人请填写密钥。更换 Webhook 时同时填写新机器人的密钥。") }>
        <input aria-label={`${name} ${t("机器人签名密钥")}`} type="password" autoComplete="off" spellCheck={false} disabled={busy || clearSecret}
          placeholder={status.hasSecret ? t("已保存，留空保留") : t("可选")} value={secret} onChange={event => setSecret(event.target.value)} />
        {status.hasSecret && <label><input type="checkbox" checked={clearSecret} disabled={busy} onChange={event => { setClearSecret(event.target.checked); setSecret(""); }} /> {t("清除签名")}</label>}
      </CxRow>
      <CxRow title={t("消息中的会话链接")} detail={t("手机需能访问所选网络，审批使用已配对设备的完全控制权限。") }>
        <AppSelect<Network> size="sm" ariaLabel={t("消息中的会话链接")} value={network} disabled={busy} onChange={setNetwork}
          options={[{ value: "lan", label: t("局域网") }, { value: "tailscale", label: "Tailscale" }, { value: "none", label: t("仅通知") }]} />
      </CxRow>
      <CxRow title={t("机器人配置")}>
        {status.hasWebhook && <button className="mg-button" type="button" disabled={busy} onClick={() => void clear()}>{t("清除")}</button>}
        <button className="mg-button" type="button" disabled={busy || (!status.hasWebhook && !webhook.trim())} onClick={() => void save()}>{t("保存")}</button>
      </CxRow>
    </>}
  </CxCard>;
}

export function ChatRobotSettings({ running, lanRunning }: { running: boolean; lanRunning: boolean }) {
  const [robots, setRobots] = useState<RobotStatus | null>(null);
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  useEffect(() => {
    let mounted = true;
    request("status").then(value => { if (mounted) setRobots(value); }).catch(reason => { if (mounted) setError(String(reason)); });
    return () => { mounted = false; };
  }, []);
  const run = async (action: "save" | "test" | "clear", provider: Provider, update?: Record<string, unknown>) => {
    setBusy(true);
    setError("");
    try { setRobots(await request(action, provider, update)); return true; }
    catch (reason) { setError(String(reason)); return false; }
    finally { setBusy(false); }
  };
  return <CxSection title={t("钉钉与飞书通知")}>
    <p>{t("任务完成、出错或等待审批时，向选定的群发送状态和会话短编号。启用局域网访问后，可从通知打开会话处理审批。")}</p>
    {!running && <CxBanner tone="warn" title={t("打开远程控制后开始监听任务通知。")}/>}
    {running && !lanRunning && <CxBanner tone="info" title={t("要在手机上打开会话，请启用局域网访问并配对设备。")}/>}
    {error && <div role="alert"><CxBanner tone="warn" title={error} /></div>}
    {robots && (["dingtalk", "feishu"] as const).map(provider => <RobotForm key={provider} provider={provider} status={robots[provider]} busy={busy} run={run} />)}
  </CxSection>;
}
