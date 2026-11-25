/** 远程控制：在浏览器 / 手机里查看与操控 Codex 桌面里的会话（发消息、插话、停止、审批、回答提问、改模型与权限）。 */
import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { AlertTriangle, Copy, ExternalLink, KeyRound, QrCode, Smartphone, Trash2, X } from "lucide-react";
import { AppSelect } from "./AppSelect";
import { ChatRobotSettings } from "./ChatRobotSettings";
import { appConfirm } from "./app-dialog";
import { Switch } from "./gateway-ui";
import { CxBadge, CxBanner, CxCard, CxRow, CxSection } from "./cx-layout";
import { t, tf } from "./i18n";
import { errorText } from "./lib/utils";
import { keepIfSame, usePolling } from "./use-polling";

export type DeviceRole = "view" | "message" | "full";

export type RemoteDevice = {
  id: string;
  name: string;
  role: DeviceRole;
  createdAt: number;
  lastSeenAt?: number | null;
  lastAddress?: string | null;
};

export type RemoteControlStatus = {
  enabled: boolean;
  weixinFocusEnabled: boolean;
  running: boolean;
  port: number;
  url?: string | null;
  error?: string | null;
  lan?: {
    enabled: boolean;
    running: boolean;
    port: number;
    addresses: { address: string; kind: "lan" | "tailscale"; url: string }[];
    fingerprint?: string | null;
    error?: string | null;
  };
  pairing?: {
    code: string;
    role: DeviceRole;
    expiresAt: number;
    links: { address: string; kind: "lan" | "tailscale"; url: string; qrSvg?: string | null }[];
  } | null;
  devices?: RemoteDevice[];
};

export type RemoteControlAction =
  | "status"
  | "enable"
  | "disable"
  | "setWeixinFocus"
  | "resetToken"
  | "enableLan"
  | "disableLan"
  | "startPairing"
  | "cancelPairing"
  | "setDeviceRole"
  | "revokeDevice";

export function remoteControlRequest(action: RemoteControlAction, args?: Record<string, string | boolean>) {
  return invoke<RemoteControlStatus>("remote_control_request", { action, args });
}

const roleLabels: Record<DeviceRole, () => string> = {
  view: () => t("只能查看"),
  message: () => t("可发消息"),
  full: () => t("完全控制"),
};

const roleHints: Record<DeviceRole, () => string> = {
  view: () => t("看会话和输出"),
  message: () => t("还能发消息、插话、停止、新建会话"),
  full: () => t("还能审批命令、回答提问、改模型和权限"),
};

function RoleSelect({ value, disabled, onChange }: { value: DeviceRole; disabled?: boolean; onChange: (role: DeviceRole) => void }) {
  return (
    <AppSelect<DeviceRole>
      size="sm"
      menuWidth={240}
      value={value}
      disabled={disabled}
      ariaLabel={t("权限")}
      options={(Object.keys(roleLabels) as DeviceRole[]).map((role) => ({ value: role, label: roleLabels[role](), hint: roleHints[role]() }))}
      onChange={onChange}
    />
  );
}

function relativeTime(seconds?: number | null) {
  if (!seconds) return t("从未");
  const diff = Date.now() / 1000 - seconds;
  if (diff < 90) return t("刚刚");
  if (diff < 3600) return tf("{0} 分钟前", [Math.floor(diff / 60)]);
  if (diff < 86400) return tf("{0} 小时前", [Math.floor(diff / 3600)]);
  return new Date(seconds * 1000).toLocaleDateString();
}

export function RemoteControlPage() {
  const [status, setStatus] = useState<RemoteControlStatus | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);
  const [pairRole, setPairRole] = useState<DeviceRole>("message");
  const [now, setNow] = useState(() => Date.now() / 1000);
  const [notice, setNotice] = useState<string | null>(null);
  const deviceCount = useRef<number | null>(null);

  const run = useCallback(async (action: RemoteControlAction, args?: Record<string, string | boolean>) => {
    setBusy(true);
    setError(null);
    try {
      setStatus(await remoteControlRequest(action, args));
    } catch (reason) {
      setError(errorText(reason));
      try {
        setStatus(await remoteControlRequest("status"));
      } catch {}
    } finally {
      setBusy(false);
    }
  }, []);

  useEffect(() => {
    void run("status");
  }, [run]);

  // 配对进行中：每 2 秒读一次状态，手机配上后立刻出现在设备列表里（窗口不可见时暂停）；
  // 倒计时单独每秒走一格，到期自动收起二维码。
  const pairing = status?.pairing ?? null;
  usePolling(async () => {
    const next = await remoteControlRequest("status");
    setStatus((current) => keepIfSame(current, next));
  }, 2000, { enabled: Boolean(pairing), immediate: false });
  usePolling(() => setNow(Date.now() / 1000), 1000, { enabled: Boolean(pairing) });

  const devices = status?.devices ?? [];
  useEffect(() => {
    if (deviceCount.current != null && devices.length > deviceCount.current) {
      setNotice(tf("已配对：{0}", [devices[devices.length - 1]?.name ?? ""]));
      setTimeout(() => setNotice(null), 4000);
    }
    deviceCount.current = devices.length;
  }, [devices.length]);

  const url = status?.url ?? null;
  const lan = status?.lan;
  const copy = async () => {
    if (!url) return;
    try {
      await navigator.clipboard.writeText(url);
    } catch (reason) {
      setError(tf("复制失败：{0}", [errorText(reason)]));
      return;
    }
    setCopied(true);
    setTimeout(() => setCopied(false), 1500);
  };
  const openInBrowser = async () => {
    if (!url) return;
    try {
      await invoke("open_external_url", { url });
    } catch (reason) {
      setError(errorText(reason));
    }
  };
  // 更换令牌会让所有已打开的网页立刻失效，先确认。
  const resetToken = async () => {
    const confirmed = await appConfirm(t("之前打开的页面会立刻失效，需要用新链接重新打开。"), {
      title: t("更换访问令牌？"),
      confirmText: t("更换"),
      danger: true,
    });
    if (confirmed) void run("resetToken");
  };
  const revoke = async (device: RemoteDevice) => {
    const confirmed = await appConfirm(t("移除后这台设备立刻无法访问，要再用需重新配对。"), {
      title: tf("移除设备「{0}」", [device.name]),
      confirmText: t("移除"),
      danger: true,
    });
    if (confirmed) void run("revokeDevice", { id: device.id });
  };
  const secondsLeft = pairing ? Math.max(0, Math.round(pairing.expiresAt - now)) : 0;

  return (
    <div className="remote-control-page">
      {error || status?.error || lan?.error ? (
        <div role="alert"><CxBanner icon={<AlertTriangle />} title={error ?? status?.error ?? lan?.error ?? ""} tone="warn" /></div>
      ) : null}
      {notice ? <div role="status"><CxBanner icon={<Smartphone />} title={notice} tone="ok" /></div> : null}
      {/* 页面标题和「实验」标记已在顶栏，这一节只叫「浏览器访问」。 */}
      <CxSection title={t("浏览器访问")}>
        <CxCard>
          <CxRow
            title={t("启用远程控制")}
            detail={t("在浏览器里查看 Codex 的会话、发消息、停止，处理命令审批和模型提问，切换模型、权限和计划模式。") + (status?.weixinFocusEnabled && !status.running ? ` ${t("开启远程控制会中断正在独立后台运行的微信专注轮次。")}` : "")}
            badges={status?.running ? <CxBadge tone="ok">{t("运行中")}</CxBadge> : null}
          >
            <Switch
              label={t("启用远程控制")}
              on={Boolean(status?.enabled)}
              pending={busy}
              disabled={busy || !status}
              onToggle={() => void run(status?.enabled ? "disable" : "enable")}
            />
          </CxRow>
          <CxRow
            title={t("允许微信专注操作桌面会话")}
            detail={t("远程控制关闭时，仅在此项开启后，已扫码绑定的微信用户仍可进入桌面会话、发消息和处理审批。不会开启网页访问。")}
          >
            <Switch
              label={t("允许微信专注操作桌面会话")}
              on={Boolean(status?.weixinFocusEnabled)}
              pending={busy}
              disabled={busy || !status}
              onToggle={() => void run("setWeixinFocus", { enabled: !status?.weixinFocusEnabled })}
            />
          </CxRow>
          {url ? (
            <CxRow
              title={t("本机访问链接")}
              detail={<code className="remote-control-url">{url.replace(/#token=.*/, "#token=…")}</code>}
              tooltip={t("链接里带着访问令牌，拿到链接就能操控 Codex，不要发给别人。")}
            >
              <button className="mg-button" type="button" onClick={() => void copy()}>
                <Copy size={14} />{copied ? t("已复制") : t("复制")}
              </button>
              <button className="mg-button" type="button" onClick={() => void openInBrowser()}>
                <ExternalLink size={14} />{t("在浏览器打开")}
              </button>
            </CxRow>
          ) : null}
          {status?.enabled ? (
            <CxRow
              title={t("更换令牌")}
              detail={t("之前打开的页面会立刻失效，需要用新链接重新打开。")}
            >
              <button className="mg-button" type="button" disabled={busy} onClick={() => void resetToken()}>
                <KeyRound size={14} />{t("更换")}
              </button>
            </CxRow>
          ) : null}
        </CxCard>
      </CxSection>
      {status?.enabled ? (
        <CxSection title={t("手机与其它设备")}>
          <CxCard>
            <CxRow
              title={t("允许局域网访问")}
              detail={t("同一 Wi-Fi 或同一 Tailscale 网络里的设备经 HTTPS 访问，只有配对过的设备能用。")}
              badges={lan?.running ? <CxBadge tone="ok">{tf("端口 {0}", [lan.port])}</CxBadge> : null}
            >
              <Switch
                label={t("允许局域网访问")}
                on={Boolean(lan?.enabled)}
                pending={busy}
                disabled={busy}
                onToggle={() => void run(lan?.enabled ? "disableLan" : "enableLan")}
              />
            </CxRow>
            {lan?.running ? (
              <>
                <CxRow
                  title={t("访问地址")}
                  detail={
                    lan.addresses.length ? (
                      <span className="remote-control-addresses">
                        {lan.addresses.map((address) => (
                          <span key={address.url}>
                            <code>{address.url}</code>
                            <CxBadge tone={address.kind === "tailscale" ? "ok" : "neutral"}>{address.kind === "tailscale" ? "Tailscale" : t("局域网")}</CxBadge>
                          </span>
                        ))}
                      </span>
                    ) : t("没有找到可用的网络地址，检查一下是否连着 Wi-Fi。")
                  }
                />
                {lan.fingerprint ? (
                  <CxRow
                    title={t("证书指纹")}
                    detail={<code className="remote-control-fingerprint">{lan.fingerprint}</code>}
                    tooltip={t("证书是这台电脑自己签的，手机第一次打开会提示不受信任。可以在浏览器的证书详情里核对这串指纹，一致再继续访问。")}
                  />
                ) : null}
                <CxRow
                  title={t("添加设备")}
                  detail={t("生成一次性配对码（5 分钟内有效），用手机相机扫码打开，确认后即配对完成。")}
                >
                  <RoleSelect value={pairRole} disabled={busy} onChange={setPairRole} />
                  <button className="mg-button" type="button" disabled={busy} onClick={() => void run("startPairing", { role: pairRole })}>
                    <QrCode size={14} />{pairing ? t("重新生成") : t("生成配对码")}
                  </button>
                </CxRow>
                {pairing && secondsLeft > 0 ? (
                  <div className="remote-control-pairing">
                    <div className="remote-control-pairing-head">
                      <span>
                        {t("配对码")} <strong className="remote-control-code">{pairing.code}</strong>
                        {" · "}{roleLabels[pairing.role]()}{" · "}{`${Math.floor(secondsLeft / 60)}:${String(secondsLeft % 60).padStart(2, "0")}`}
                      </span>
                      <button className="mg-button" type="button" onClick={() => void run("cancelPairing")}>
                        <X size={14} />{t("取消")}
                      </button>
                    </div>
                    <div className="remote-control-qrs">
                      {pairing.links.map((link) => (
                        <figure key={link.url}>
                          {link.qrSvg ? <div className="remote-control-qr" dangerouslySetInnerHTML={{ __html: link.qrSvg }} /> : null}
                          <figcaption>
                            {link.kind === "tailscale" ? "Tailscale" : t("局域网")} · <code>{link.address}</code>
                          </figcaption>
                        </figure>
                      ))}
                    </div>
                    <p className="remote-control-hint">
                      {t("扫不了码时，在手机浏览器打开上面的访问地址，再输入配对码。")}
                    </p>
                  </div>
                ) : null}
              </>
            ) : null}
          </CxCard>
        </CxSection>
      ) : null}
      {devices.length ? (
        <CxSection count={devices.length} title={t("已配对设备")}>
          <CxCard>
            {devices.map((device) => (
              <CxRow
                key={device.id}
                icon={<Smartphone size={16} />}
                title={device.name}
                detail={device.lastAddress
                  ? tf("最近活动 {0} · {1}", [relativeTime(device.lastSeenAt), device.lastAddress])
                  : tf("最近活动 {0}", [relativeTime(device.lastSeenAt)])}
              >
                <RoleSelect value={device.role} disabled={busy} onChange={(role) => void run("setDeviceRole", { id: device.id, role })} />
                <button className="mg-button" type="button" disabled={busy} onClick={() => void revoke(device)}>
                  <Trash2 size={14} />{t("移除")}
                </button>
              </CxRow>
            ))}
          </CxCard>
        </CxSection>
      ) : null}
      <ChatRobotSettings running={Boolean(status?.running)} lanRunning={Boolean(lan?.running)} />
      <CxSection title={t("使用说明")}>
        <CxCard>
          <CxRow
            title={t("需要从 Codex++ 启动 Codex")}
            detail={t("会话在 Codex 桌面里执行。没在桌面打开的会话，发消息时会自动在桌面打开它，这一步需要 Codex 由 Codex++ 启动。")}
          />
          <CxRow
            title={t("审批与提问")}
            detail={t("命令审批、模型提问会同时出现在桌面和网页上，任一端处理即可。")}
          />
          <CxRow
            title={t("不在同一网络时")}
            detail={t("在电脑和手机上都装 Tailscale 并登录同一账号，就能用 Tailscale 地址随时访问，不经过任何第三方服务器。")}
          />
          <CxRow
            title={t("提醒")}
            detail={t("网页开着时，需要审批、模型提问或一轮跑完会用标题、振动和系统通知提醒。网页关掉后收不到：自签证书下浏览器不允许后台推送。")}
          />
        </CxCard>
      </CxSection>
    </div>
  );
}
