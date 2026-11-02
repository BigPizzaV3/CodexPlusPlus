import { useEffect, useRef, useState } from "react";
import "./flow-demo.css";

type Stage = "idle" | "unrouted" | "waiting" | "generating" | "mixed" | "done" | "error";
const stages: Array<{ id: Stage; label: string; description: string }> = [
  { id: "idle", label: "空闲", description: "没有进行中的请求。历史成功、失败记录不会在图上重新播放。" },
  { id: "unrouted", label: "路由选择中", description: "尚未选定供应商，只显示 Codex 到 Codex++ 的去程。" },
  { id: "waiting", label: "发送 / 等待", description: "拖尾光点从 Codex 出发，经过 Codex++，流向 ChatGPT Pro。" },
  { id: "generating", label: "响应回流", description: "沿同一条完整路线反向流动：ChatGPT Pro → Codex++ → Codex。" },
  { id: "mixed", label: "双向并发", description: "ChatGPT Pro 正在响应，另一条请求正在发往 jojocodemax；共用连线也保留各自方向。" },
  { id: "done", label: "完成", description: "请求成功后立即停止光流。成功结果留在对应请求行，不再留下游离绿点。" },
  { id: "error", label: "失败", description: "请求失败后立即停止光流。失败原因留在对应请求行，不再在其他链路播放红点。" },
];

export function FlowDemo() {
  const [stage, setStage] = useState<Stage>("waiting");
  const [playing, setPlaying] = useState(false);
  const [loaded, setLoaded] = useState(false);
  const frame = useRef<HTMLIFrameElement>(null);
  const startedAt = useRef(Date.now());
  const timers = useRef<Array<ReturnType<typeof setTimeout>>>([]);
  const initial = useRef(`/?flowPreview=waiting&flowStart=${startedAt.current}&lang=zh#tray-panel`);
  const stop = () => { timers.current.forEach(clearTimeout); timers.current = []; setPlaying(false); };

  useEffect(() => {
    // 演示只修改此预览站点的存储，不访问真实应用设置。
    localStorage.setItem("codex-plus-tray-tab", "flow");
    return () => timers.current.forEach(clearTimeout);
  }, []);

  useEffect(() => {
    if (!loaded) return;
    const target = frame.current?.contentWindow;
    target?.history.replaceState(null, "", `/?flowPreview=${stage}&flowStart=${startedAt.current}&lang=zh#tray-panel`);
  }, [stage, loaded]);

  const select = (id: Stage) => { stop(); startedAt.current = Date.now(); setStage(id); };
  const play = () => {
    stop(); startedAt.current = Date.now(); setStage("idle"); setPlaying(true);
    const sequence: Array<[number, Stage]> = [[1200, "unrouted"], [3500, "waiting"], [7500, "generating"], [13500, "mixed"], [19500, "done"]];
    timers.current = sequence.map(([delay, id]) => setTimeout(() => { setStage(id); if (id === "done") setPlaying(false); }, delay));
  };
  const current = stages.find((item) => item.id === stage)!;

  return <main className="flow-demo">
    <section className="flow-demo-controls">
      <div className="flow-demo-eyebrow">CODEX++ / REQUEST FLOW LAB</div>
      <h1>真实状态，连续光流。</h1>
      <p className="flow-demo-intro">这是模拟网页，复用应用里的真实托盘组件与动画。所有请求均为本地测试数据，不调用模型、不消耗额度。</p>
      <div className="flow-demo-actions">
        <button className="flow-demo-primary" onClick={play}>{playing ? "重新播放完整流程" : "播放完整流程"}</button>
        {playing ? <button onClick={stop}>暂停状态切换</button> : null}
      </div>
      <div className="flow-demo-stages" role="group" aria-label="模拟请求状态">
        {stages.map((item) => <button key={item.id} data-stage={item.id} aria-pressed={stage === item.id} onClick={() => select(item.id)}>{item.label}</button>)}
      </div>
      <section className="flow-demo-description" aria-live="polite"><h2>{current.label}</h2><p>{current.description}</p></section>
      <dl className="flow-demo-notes">
        <div><dt>看归属</dt><dd>每家供应商固定一种颜色，光流、供应商节点和请求行色点一致；同一家去回同色。</dd></div>
        <div><dt>看方向</dt><dd>亮头在前，淡尾在后；去程和回程不再只是同样的闪烁虚线。</dd></div>
        <div><dt>看连续</dt><dd>整个路线共用时钟和速度，经过 Codex++ 节点时不重新起跑。</dd></div>
        <div><dt>看结束</dt><dd>完成、失败后立即停止；历史结果只在请求列表展示。</dd></div>
      </dl>
      <p className="flow-demo-footnote">状态切换最多约 1 秒同步到托盘。若系统开启“减弱动态效果”，保留活动线路、关闭移动光点。</p>
    </section>
    <section className="flow-demo-preview" aria-label="托盘模拟预览">
      <header><span>模拟数据 · 与应用共用组件</span><span>380 × 620</span></header>
      <iframe ref={frame} src={initial.current} title="Codex++ 实时请求模拟托盘" onLoad={() => setLoaded(true)} />
    </section>
  </main>;
}
