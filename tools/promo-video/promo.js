// Codex++ 宣传片：整片是时间 t 的纯函数，window.__seek(t) 把画面摆到第 t 秒。
// 渲染脚本逐帧调用 __seek 截图；直接用浏览器打开则可预览播放（带配乐）。
(() => {
  const DURATION = 120;
  const $ = (sel, root = document) => root.querySelector(sel);
  const $$ = (sel, root = document) => [...root.querySelectorAll(sel)];
  const clamp = (x, a = 0, b = 1) => Math.min(b, Math.max(a, x));
  const lin = (t, a, b) => clamp((t - a) / (b - a));
  const mix = (a, b, p) => a + (b - a) * p;
  const E = {
    lin: x => x,
    out3: x => 1 - (1 - x) ** 3,
    in3: x => x ** 3,
    io3: x => (x < 0.5 ? 4 * x ** 3 : 1 - (-2 * x + 2) ** 3 / 2),
    expo: x => (x >= 1 ? 1 : 1 - 2 ** (-10 * x)),
    back: x => { const c1 = 1.70158, c3 = c1 + 1; return 1 + c3 * (x - 1) ** 3 + c1 * (x - 1) ** 2; },
  };
  const ease = (t, a, b, f = E.out3) => f(lin(t, a, b));
  const hash = n => { const x = Math.sin(n * 127.1 + 311.7) * 43758.5453; return x - Math.floor(x); };
  const typed = (s, p) => { const chars = Array.from(s); return chars.slice(0, Math.round(chars.length * clamp(p))).join(""); };
  const div = (cls, html = "") => { const el = document.createElement("div"); if (cls) el.className = cls; el.innerHTML = html; return el; };

  // 统一设置变换：o 透明度，x/y/z 位移，s 缩放，rx/ry/rz 旋转，blur 模糊。
  function put(el, { o, x = 0, y = 0, z = 0, s = 1, sx, rx = 0, ry = 0, rz = 0, blur = 0 } = {}) {
    let tr = `translate3d(${x.toFixed(2)}px,${y.toFixed(2)}px,${z.toFixed(2)}px)`;
    if (rx) tr += ` rotateX(${rx.toFixed(3)}deg)`;
    if (ry) tr += ` rotateY(${ry.toFixed(3)}deg)`;
    if (rz) tr += ` rotateZ(${rz.toFixed(3)}deg)`;
    if (sx != null) tr += ` scale(${sx.toFixed(4)}, ${s.toFixed(4)})`;
    else if (s !== 1) tr += ` scale(${s.toFixed(4)})`;
    el.style.transform = tr;
    if (o != null) { el.style.opacity = clamp(o).toFixed(3); el.style.visibility = o <= 0.001 ? "hidden" : "visible"; }
    el.style.filter = blur > 0.05 ? `blur(${blur.toFixed(2)}px)` : "none";
  }
  // 入场 / 出场的透明度包络。
  const env = (t, a, b, fin = 0.3, fout = 0.3) => Math.min(ease(t, a, a + fin), 1 - ease(t, b - fout, b, E.in3));

  // 音乐段落：满编段才有鼓点脉冲（和 music.mjs 一致）。
  const FULL = [[8, 64], [68, 108]];
  const inFull = t => FULL.some(([a, b]) => t >= a && t < b);
  const pulse = t => (inFull(t) ? Math.exp(-(t % 0.5) / 0.12) : 0);

  let REG = {};
  const R = (shot, key) => REG[shot]?.[key];
  const center = r => ({ cx: r.x + r.w / 2, cy: r.y + r.h / 2 });

  // —————————— 背景：网格地面、光晕、星点 ——————————
  const canvas = $("#bg");
  const g2 = canvas.getContext("2d");
  const GRID = [[0, 0], [6, 0.35], [7.9, 0.6], [8, 1], [11, 0.7], [12, 0.75], [20, 0.75], [20.01, 1], [21.5, 1], [21.51, 0.12], [32, 0.15], [32.01, 0.35], [40, 0.35], [40.01, 1], [41.5, 1], [41.51, 0.28], [52, 0.28], [52.01, 0.1], [64, 0.1], [64.01, 1], [65.5, 1], [65.51, 0.22], [76, 0.22], [76.01, 1], [77.5, 1], [77.51, 0.1], [92, 0.1], [92.01, 0.3], [108, 0.4], [108.01, 0.9], [117, 0.6], [120, 0]];
  const curve = (table, t) => {
    for (let i = 1; i < table.length; i++) if (t <= table[i][0]) return mix(table[i - 1][1], table[i][1], lin(t, table[i - 1][0], table[i][0]));
    return table.at(-1)[1];
  };
  function orb(x, y, r, color) {
    const g = g2.createRadialGradient(x, y, 0, x, y, r);
    g.addColorStop(0, color); g.addColorStop(1, "rgba(0,0,0,0)");
    g2.fillStyle = g; g2.fillRect(x - r, y - r, r * 2, r * 2);
  }
  function drawBg(t) {
    const gi = curve(GRID, t);
    const p = pulse(t);
    g2.globalCompositeOperation = "source-over";
    g2.fillStyle = "#05060b"; g2.fillRect(0, 0, 1920, 1080);
    g2.globalCompositeOperation = "lighter";
    orb(960 + 520 * Math.sin(t * 0.13), 330 + 120 * Math.cos(t * 0.17), 900, `rgba(255,106,43,${(0.07 + 0.05 * gi + 0.03 * p).toFixed(3)})`);
    orb(380 + 300 * Math.cos(t * 0.11), 820, 820, "rgba(156,134,255,0.11)");
    orb(1620, 220 + 110 * Math.sin(t * 0.2), 720, "rgba(123,167,255,0.08)");
    // 星点
    for (let i = 0; i < 150; i++) {
      const x = (hash(i) * 1920 + t * (8 + hash(i + 9) * 30)) % 1920;
      const y = hash(i + 3) * 700;
      const a = (0.15 + 0.5 * hash(i + 5)) * (0.6 + 0.4 * Math.sin(t * (1 + hash(i + 7) * 3) + i));
      g2.fillStyle = `rgba(200,210,255,${(a * (0.4 + gi * 0.6)).toFixed(3)})`;
      const s = 1 + hash(i + 11) * 1.8;
      g2.fillRect(x, y, s, s);
    }
    // 透视网格地面
    if (gi > 0.01) {
      const hy = 650, vx = 960;
      const a0 = gi * (0.5 + 0.5 * p);
      g2.lineWidth = 1.5;
      for (let i = -26; i <= 26; i++) {
        const alpha = a0 * 0.35 * (1 - Math.abs(i) / 30);
        g2.strokeStyle = `rgba(156,134,255,${alpha.toFixed(3)})`;
        g2.beginPath(); g2.moveTo(vx + i * 10, hy); g2.lineTo(vx + i * 230, 1080); g2.stroke();
      }
      const speed = 2.2;
      for (let n = 0; n < 22; n++) {
        const z = 22 - ((n + t * speed) % 22);
        if (z < 0.6) continue;
        const y = hy + 430 / z;
        const alpha = a0 * 0.55 * clamp(1 - z / 22) ** 1.2;
        g2.strokeStyle = `rgba(255,120,70,${alpha.toFixed(3)})`;
        g2.beginPath(); g2.moveTo(0, y); g2.lineTo(1920, y); g2.stroke();
      }
      const hg = g2.createLinearGradient(0, hy - 60, 0, hy + 40);
      hg.addColorStop(0, "rgba(255,106,43,0)"); hg.addColorStop(0.6, `rgba(255,106,43,${(a0 * 0.35).toFixed(3)})`); hg.addColorStop(1, "rgba(255,106,43,0)");
      g2.fillStyle = hg; g2.fillRect(0, hy - 60, 1920, 100);
      // 地面靠近镜头处压暗，免得抢戏
      g2.globalCompositeOperation = "source-over";
      const fg = g2.createLinearGradient(0, hy, 0, 1080);
      fg.addColorStop(0, "rgba(5,6,11,0)"); fg.addColorStop(1, "rgba(5,6,11,0.55)");
      g2.fillStyle = fg; g2.fillRect(0, hy, 1920, 430);
    }
    g2.globalCompositeOperation = "source-over";
  }

  // —————————— 截图窗口 ——————————
  function makeWin(name, w, { base = [1440, 900], title = "Codex++" } = {}) {
    const k = w / base[0];
    const bh = base[1] * k;
    const el = div("win");
    el.style.width = `${w}px`;
    el.innerHTML = `<div class="win-bar"><i></i><i></i><i></i><span>${title}</span></div><div class="win-body" style="width:${w}px;height:${bh}px"><div class="cam" style="width:${base[0]}px;height:${base[1]}px"><img src="/shots/${name}.png" style="width:${base[0]}px;height:${base[1]}px"></div></div>`;
    return { el, k, w, bh, h: bh + 36, base, cam: $(".cam", el), state: { tx: 0, ty: 0, Z: k } };
  }
  function setCam(win, { cx, cy, z = 1 }) {
    const [W0, H0] = win.base;
    cx ??= W0 / 2; cy ??= H0 / 2;
    const Z = z * win.k;
    const tx = clamp(win.w / 2 - Z * cx, win.w - Z * W0, 0);
    const ty = clamp(win.bh / 2 - Z * cy, win.bh - Z * H0, 0);
    win.cam.style.transform = `translate(${tx.toFixed(2)}px,${ty.toFixed(2)}px) scale(${Z.toFixed(4)})`;
    win.state = { tx, ty, Z };
  }
  // 镜头关键帧：[[时刻, {cx, cy, z}], ...]，相邻关键帧之间缓动插值。
  function camAt(lt, keys) {
    if (lt <= keys[0][0]) return keys[0][1];
    for (let i = 1; i < keys.length; i++) {
      if (lt <= keys[i][0]) {
        const p = E.io3(lin(lt, keys[i - 1][0], keys[i][0]));
        const a = keys[i - 1][1], b = keys[i][1];
        const full = { cx: 720, cy: 450, z: 1 };
        return { cx: mix(a.cx ?? full.cx, b.cx ?? full.cx, p), cy: mix(a.cy ?? full.cy, b.cy ?? full.cy, p), z: mix(a.z ?? 1, b.z ?? 1, p) };
      }
    }
    return keys.at(-1)[1];
  }
  const mapRect = (win, left, top, r) => ({ x: left + win.state.tx + r.x * win.state.Z, y: top + 36 + win.state.ty + r.y * win.state.Z, w: r.w * win.state.Z, h: r.h * win.state.Z });

  function drawMark(mark, tip, rect, o, { place = "above", pad = 8 } = {}) {
    const s = mix(1.12, 1, E.out3(clamp(o * 1.4)));
    const x = rect.x - pad, y = rect.y - pad, w = rect.w + pad * 2, h = rect.h + pad * 2;
    mark.style.left = `${x}px`; mark.style.top = `${y}px`; mark.style.width = `${w}px`; mark.style.height = `${h}px`;
    put(mark, { o, s });
    if (tip) {
      const tw = tip.offsetWidth;
      const left = clamp(x, 40, 1880 - tw);
      tip.style.left = `${left}px`;
      tip.style.top = `${place === "above" ? y - 66 : y + h + 16}px`;
      put(tip, { o, y: mix(14, 0, E.out3(clamp(o))) });
    }
  }

  // —————————— 场景注册 ——————————
  const scenes = [];
  const scene = (el, t0, t1, update, display = "block") => scenes.push({ el, t0, t1, update, display });

  // 1. 开场 ——————————————————————————————
  {
    const root = $("#s-intro");
    const l1 = $(".l1 .tx", root), l2 = $(".l2 .tx", root), term = $(".term", root), caret = $(".term .caret", root);
    const brand = $(".brand", root), logo = $(".logo", brand), codex = $(".codex", brand), pp = $(".pp", brand), tag = $(".tag", brand), sub = $(".sub", brand);
    const rings = $$(".ring", brand);
    scene(root, 0, 12, lt => {
      l1.textContent = typed("你的 AI 编程 Agent，", lin(lt, 0.6, 2.4));
      l2.textContent = typed("还能再顺手一点吗？", lin(lt, 3.2, 5.0));
      // 光标跟着正在打字的那一行；两行都还空时留在第一行。
      const line = lt < 3.2 ? l1.parentElement : l2.parentElement;
      if (caret.parentElement !== line) line.append(caret);
      $(".l2", root).style.visibility = lt < 3.2 ? "hidden" : "visible";
      caret.style.opacity = Math.floor(lt * 2.4) % 2 === 0 || (lt > 0.6 && lt < 5.0) ? 1 : 0;
      const out = ease(lt, 6.2, 7.9, E.in3);
      put(term, { o: 1 - out, s: 1 + out * 0.9, blur: out * 18, y: -out * 40 });
      term.style.textShadow = `${(out * 14).toFixed(1)}px 0 rgba(255,60,90,.8), ${(-out * 14).toFixed(1)}px 0 rgba(80,160,255,.8)`;
      term.style.display = lt < 8 ? "block" : "none";
      brand.style.display = lt >= 8 ? "flex" : "none";
      if (lt >= 8) {
        const b = lt - 8;
        const exit = ease(lt, 11.4, 12, E.in3);
        put(logo, { o: ease(b, 0, 0.12) * (1 - exit), s: mix(2.8, 1, ease(b, 0, 0.5, E.expo)) * (1 + 0.03 * pulse(lt)), rz: mix(-16, 0, ease(b, 0, 0.6, E.expo)) });
        rings.forEach((ring, i) => {
          const p = lin(b, i * 0.12, 1.1 + i * 0.12);
          put(ring, { o: (1 - p) * (p > 0 ? 1 : 0) * 0.9, s: mix(0.3, 3.2 + i * 0.6, E.out3(p)) });
        });
        put(codex, { o: ease(b, 0.15, 0.4) * (1 - exit), x: mix(-260, 0, ease(b, 0.15, 0.7, E.expo)), blur: mix(20, 0, ease(b, 0.15, 0.6)) });
        put(pp, { o: ease(b, 0.3, 0.5) * (1 - exit), x: mix(300, 0, ease(b, 0.3, 0.85, E.back)), s: mix(1.6, 1, ease(b, 0.3, 0.85, E.back)) });
        put(tag, { o: ease(b, 1.0, 1.5) * (1 - exit), y: mix(30, 0, ease(b, 1.0, 1.6)) });
        put(sub, { o: ease(b, 1.5, 2.0) * (1 - exit), y: mix(30, 0, ease(b, 1.5, 2.1)) });
        put(brand, { s: mix(1, 1.04, lin(b, 0, 4)) * (1 + exit * 0.3), blur: exit * 12 });
      }
    });
  }

  // 2. Agent 快切 ————————————————————————
  {
    const root = $("#s-agents");
    const names = ["Codex", "Claude Code", "Claude Desktop", "Grok", "Gemini CLI", "OpenCode", "Zed", "Qwen Code", "Copilot CLI", "Crush", "Goose", "Cline", "Droid", "Kimi Code", "Qoder CLI", "CodeBuddy", "WorkBuddy", "DeepSeek Harness", "Raycast", "JetBrains Air", "MiniMax Code", "Hermes Agent", "OpenClaw", "VS Code Chat", "T3 Code", "Snow CLI", "AtomCode", "StepCode", "MiMo Code", "Reasonix Studio", "ZCode", "AstrBot", "OpenChamber", "Pencil", "Aside", "Pi"];
    const marquee = $(".marquee", root);
    const rows = [0, 1, 2, 3, 4, 5].map(r => {
      const row = div("row");
      const list = names.slice(r * 6).concat(names.slice(0, r * 6));
      row.innerHTML = [...list, ...list].map(n => `<span>${n}</span>`).join("");
      marquee.append(row);
      return row;
    });
    const words = ["Codex", "Claude Code", "Claude Desktop", "Grok", "Gemini CLI", "OpenCode", "Zed", "Qwen Code"];
    const wrap = $(".word-wrap", root), word = $(".word:not(.ghost)", root), ghost = $(".word.ghost", root), index = $(".word-index", root);
    const big = $(".big50", root), num = $(".num", big), plus = $(".plus", big), cap = $(".cap", big);
    const unify = $(".unify", root), h2 = $("h2", unify), pills = $$(".pills span", unify);
    scene(root, 12, 20, lt => {
      rows.forEach((row, i) => put(row, { x: (i % 2 ? -1 : 1) * (lt * 90 + 40 * i) - (i % 2 ? 0 : 1600), o: 1 }));
      put(marquee, { o: 0.9 * env(lt, 0, 8, 0.4, 0.4) });
      const k = Math.floor(lt / 0.5);
      if (lt < 4) {
        wrap.style.display = "flex";
        const local = lt - k * 0.5;
        word.textContent = ghost.textContent = words[k];
        index.textContent = `接入 · ${String(k + 1).padStart(2, "0")}`;
        put(word, { o: 1, s: mix(1.28, 1, ease(local, 0, 0.16, E.expo)), y: mix(-30, 0, ease(local, 0, 0.16, E.expo)) });
        put(ghost, { o: 1 - ease(local, 0, 0.35), s: mix(1, 1.5, ease(local, 0, 0.4)) });
      } else wrap.style.display = "none";
      big.style.display = lt >= 4 && lt < 5.75 ? "flex" : "none";
      if (lt >= 4 && lt < 5.75) {
        const b = lt - 4;
        num.textContent = String(Math.round(50 * ease(b, 0, 0.7, E.out3)));
        put(num, { o: 1, s: mix(0.6, 1, ease(b, 0, 0.45, E.expo)) * (1 + 0.04 * pulse(lt + 12)) });
        put(plus, { o: ease(b, 0.55, 0.7), s: mix(2, 1, ease(b, 0.55, 0.9, E.back)), rz: mix(90, 0, ease(b, 0.55, 0.9)) });
        put(cap, { o: ease(b, 0.8, 1.1), y: mix(26, 0, ease(b, 0.8, 1.2)) });
        put(big, { o: 1 - ease(b, 1.55, 1.75), s: 1 + ease(b, 1.5, 1.75, E.in3) * 0.5, blur: ease(b, 1.5, 1.75) * 16 });
      }
      unify.style.display = lt >= 5.75 ? "flex" : "none";
      if (lt >= 5.75) {
        const b = lt - 5.75;
        put(h2, { o: ease(b, 0, 0.25), y: mix(40, 0, ease(b, 0, 0.4, E.expo)), blur: mix(10, 0, ease(b, 0, 0.3)) });
        pills.forEach((pill, i) => put(pill, { o: ease(b, 0.5 + i * 0.25, 0.75 + i * 0.25), s: mix(0.6, 1, ease(b, 0.5 + i * 0.25, 0.9 + i * 0.25, E.back)) }));
      }
    });
  }

  // 章节卡 ————————————————————————————
  const CHAPTERS = [["#s-ch1", 20], ["#s-ch2", 40], ["#s-ch3", 64], ["#s-ch4", 76]];
  for (const [sel, t0] of CHAPTERS) {
    const root = $(sel);
    root.innerHTML = `<div class="num">${root.dataset.num}</div><div class="bar"></div><div class="t">${root.dataset.title}</div><div class="s">${root.dataset.sub}</div>`;
    const [num, bar, title, sub] = [...root.children];
    scene(root, t0, t0 + 1.5, lt => {
      const out = ease(lt, 1.3, 1.5, E.in3);
      put(num, { o: ease(lt, 0, 0.2) * (1 - out), x: mix(-160, 0, ease(lt, 0, 0.55, E.expo)) + lt * 20 - out * 80, blur: out * 10 });
      put(bar, { o: 1 - out, sx: ease(lt, 0.1, 0.55, E.expo), s: 1 });
      put(title, { o: ease(lt, 0.12, 0.35) * (1 - out), y: mix(50, 0, ease(lt, 0.12, 0.6, E.expo)), x: -out * 60, blur: out * 8 });
      put(sub, { o: ease(lt, 0.3, 0.55) * (1 - out), y: mix(30, 0, ease(lt, 0.3, 0.75, E.expo)), x: -out * 60 });
    });
  }

  // 3. 对话造拓展 ————————————————————————
  {
    const root = $("#s-chat");
    const win = $(".chat-win", root);
    const body = $(".chat-body", root);
    const inner = div("inner");
    inner.append(...body.children);
    body.append(inner);
    const u1 = $(".u1", root), u2 = $(".u2", root), w1 = $(".w1", root), b1 = $(".b1 .tx", root), b2 = $(".b2 .tx", root);
    const b1m = $(".b1", root), b2m = $(".b2", root);
    const steps = $$(".step", w1), head = $(".work-head", w1);
    const input = $(".composer .tx", root), caret = $(".composer .caret", root), ph = $(".composer .ph", root);
    const send = $(".send", root), arrow = $(".arrow", send), dog = $(".dog", send), burst = $(".burst", send);
    const ring = $(".ctx-ring", root), bar = $(".ctx-bar", root);
    const cSend = $(".c-send", root), cCtx = $(".c-ctx", root), caption = $(".caption", root);
    const parts = Array.from({ length: 14 }, (_, i) => { const p = document.createElement("i"); p.style.background = ["#ffb347", "#ff6a2b", "#9c86ff", "#7ba7ff", "#3ecf8e", "#ff5c9a"][i % 6]; burst.append(p); return p; });
    const M1 = "把发送按钮改成一个🐶狗头";
    const M2 = "再把上下文圆环改成彩虹渐变进度条";
    const A1 = "已经换成 🐶，截图确认生效了。点狗头即可发送，原有功能不变。";
    const A2 = "已改成彩虹渐变进度条，截图确认生效；会随上下文用量实时更新，旁边显示百分比。";
    const show = (el, on, lt0, lt) => { el.style.display = on ? "" : "none"; if (on) put(el, { o: ease(lt, lt0, lt0 + 0.25), y: mix(26, 0, ease(lt, lt0, lt0 + 0.35, E.expo)) }); };
    const stageRect = () => $("#stage").getBoundingClientRect();
    const rel = el => { const s = stageRect(), r = el.getBoundingClientRect(), k = s.width / 1920; return { x: (r.left - s.left) / k, y: (r.top - s.top) / k, w: r.width / k, h: r.height / k }; };
    scene(root, 21.5, 32, lt => {
      const enter = ease(lt, 0, 0.7, E.expo);
      const exit = ease(lt, 10.2, 10.5, E.in3);
      put(win, { o: enter * (1 - exit), rx: mix(24, 0, enter), y: mix(160, 0, enter) - exit * 40, s: mix(0.86, 1, enter) * (1 - exit * 0.05) });
      // 输入框
      let text = "";
      if (lt >= 0.6 && lt < 2.6) text = typed(M1, lin(lt, 0.6, 2.3));
      if (lt >= 7.3 && lt < 8.6) text = typed(M2, lin(lt, 7.3, 8.3));
      input.textContent = text;
      ph.style.display = text ? "none" : "";
      caret.style.opacity = text || Math.floor(lt * 2.4) % 2 === 0 ? 1 : 0;
      // 对话
      show(u1, lt >= 2.6, 2.6, lt);
      show(w1, lt >= 3.0, 3.0, lt);
      steps.forEach((step, i) => {
        const a = 3.1 + i * 0.7;
        step.style.display = lt >= a ? "" : "none";
        step.classList.toggle("done", lt >= a + 0.7);
        $(".ico", step).style.transform = lt < a + 0.7 ? `rotate(${(lt * 720) % 360}deg)` : "none";
        if (lt >= a) put(step, { o: ease(lt, a, a + 0.2), x: mix(-20, 0, ease(lt, a, a + 0.3)) });
      });
      head.innerHTML = lt < 5.9 ? `工作中 <b>${Math.max(0, Math.floor((lt - 3) * 11))} 秒</b>` : "已完成 <b>4 步 · 写入走 Codex 工具审批</b>";
      show(b1m, lt >= 6.0, 6.0, lt);
      b1.textContent = typed(A1, lin(lt, 6.1, 7.1));
      show(u2, lt >= 8.6, 8.6, lt);
      show(b2m, lt >= 9.0, 9.0, lt);
      b2.textContent = typed(A2, lin(lt, 9.05, 9.9));
      // 内容超出就整体上移
      const avail = body.clientHeight - 34;
      const over = Math.max(0, inner.scrollHeight - avail);
      put(inner, { y: -over });
      // 发送按钮 → 狗头
      const m = ease(lt, 5.9, 6.25, E.in3);
      put(arrow, { o: 1 - m, s: 1 - m, rz: m * 180 });
      put(dog, { o: ease(lt, 6.05, 6.2), s: lt < 6.05 ? 0 : mix(0.2, 1, ease(lt, 6.05, 6.55, E.back)) * (1 + 0.08 * Math.sin(Math.max(0, lt - 6.6) * 6) * Math.exp(-Math.max(0, lt - 6.6))) });
      send.style.boxShadow = `0 0 ${(60 * Math.exp(-Math.max(0, lt - 6.05) / 0.5) * (lt > 6.05 ? 1 : 0)).toFixed(1)}px rgba(255,180,80,.9)`;
      parts.forEach((p, i) => {
        const bp = lin(lt, 6.05, 6.9);
        const ang = (i / parts.length) * Math.PI * 2 + hash(i) * 0.5;
        const r = E.out3(bp) * (70 + hash(i + 3) * 60);
        put(p, { o: bp > 0 && bp < 1 ? 1 - bp : 0, x: Math.cos(ang) * r, y: Math.sin(ang) * r, s: 1 - bp * 0.6 });
      });
      // 上下文圆环 → 彩虹进度条
      const c = ease(lt, 9.0, 9.5, E.io3);
      put(ring, { o: 1 - c, s: 1 - c * 0.5 });
      ring.style.display = c >= 1 ? "none" : "";
      bar.style.width = `${(c * 150).toFixed(1)}px`;
      // 标注
      if (lt > 6.3 && lt < 10.3) {
        const r = rel(send);
        cSend.style.left = `${r.x + r.w + 34}px`; cSend.style.top = `${r.y + r.h / 2 - 30}px`;
      }
      put(cSend, { o: env(lt, 6.4, 10.3, 0.25, 0.25), x: mix(-30, 0, ease(lt, 6.4, 6.8, E.expo)) });
      if (lt > 9.3 && lt < 10.3) {
        const r = rel(bar);
        cCtx.style.left = `${r.x - 60}px`; cCtx.style.top = `${r.y - 110}px`;
      }
      put(cCtx, { o: env(lt, 9.4, 10.3, 0.2, 0.2), y: mix(20, 0, ease(lt, 9.4, 9.8, E.expo)) });
      put(caption, { o: env(lt, 1.0, 10.3, 0.4, 0.3) * 0.95 });
    });
  }

  // 4. 拓展页轮播 + 能力标签 ———————————————
  {
    const root = $("#s-ext");
    const carousel = $(".carousel", root);
    const items = [
      { name: "ext-featured", k: "精选与官方", v: "经审核，安装锁定到审核过的提交" },
      { name: "ext-local", k: "本地拓展", v: "自己写的、对话造的，一键启停与热重载" },
      { name: "ext-community", k: "社区拓展", v: "装前列出全部能力，装完默认不启用" },
    ].map(item => { const win = makeWin(item.name, 1180); carousel.append(win.el); return { ...item, win }; });
    const label = $(".ext-label", root), lk = $(".k", label), lv = $(".v", label);
    const markLayer = div("marks"); root.append(markLayer);
    const mark = div("mark"), tip = div("tip", "对话造拓展：在 Codex 里说一句就行");
    markLayer.append(mark, tip);
    const chipNames = ["界面插槽与页面", "斜杠命令", "给模型加工具", "Codex 钩子", "网关中间件", "常驻后台服务", "消息操作与侧边面板", "配置自动生成设置表单"];
    const chipsEl = $(".chips", root);
    const chipPos = [[300, 250], [1240, 220], [180, 690], [1350, 700], [760, 170], [700, 800], [1180, 870], [230, 880]];
    const chips = chipNames.map((name, i) => { const el = document.createElement("span"); el.textContent = name; el.style.left = `${chipPos[i][0]}px`; el.style.top = `${chipPos[i][1]}px`; chipsEl.append(el); return el; });
    const chipsTitle = $(".chips-title", root);
    const RAD = 1500;
    scene(root, 32, 40, lt => {
      const theta = 32 * ease(lt, 1.9, 2.4, E.io3) + 32 * ease(lt, 3.9, 4.4, E.io3);
      const enter = ease(lt, 0, 0.6, E.expo);
      const recede = ease(lt, 5.6, 6.1, E.io3);
      items.forEach((item, i) => {
        const a = i * 32 - theta;
        const rad = a * Math.PI / 180;
        const left = 960 - item.win.w / 2, top = 520 - item.win.h / 2;
        item.win.el.style.left = `${left}px`; item.win.el.style.top = `${top}px`;
        put(item.win.el, {
          o: clamp(1 - Math.abs(a) / 70) * enter * (1 - recede * 0.8),
          x: Math.sin(rad) * RAD * 0.75,
          z: (Math.cos(rad) - 1) * RAD - mix(900, 0, enter) - recede * 1400,
          ry: -a * 0.9,
        });
        setCam(item.win, i === 0 ? camAt(lt, [[0.6, {}], [1.1, { cx: 1000, cy: 260, z: 1.35 }], [1.8, { cx: 1000, cy: 260, z: 1.35 }], [2.2, {}]]) : { z: 1 });
      });
      // 「对话造拓展」按钮高亮
      const chat = R("ext-featured", "chat");
      if (chat && lt > 1.0 && lt < 1.95) {
        const w = items[0].win;
        drawMark(mark, tip, mapRect(w, 960 - w.w / 2, 520 - w.h / 2, chat), env(lt, 1.05, 1.9, 0.2, 0.15), { place: "below", pad: 6 });
      } else { put(mark, { o: 0 }); put(tip, { o: 0 }); }
      const front = Math.round(theta / 32);
      lk.textContent = items[front].k; lv.textContent = items[front].v;
      const nearCut = Math.min(Math.abs(lt - 2.15), Math.abs(lt - 4.15));
      put(label, { o: enter * (1 - recede) * clamp(nearCut / 0.25), y: mix(20, 0, enter) });
      chips.forEach((chip, i) => {
        const a = 5.9 + i * 0.12;
        put(chip, { o: ease(lt, a, a + 0.2) * (1 - ease(lt, 7.7, 8, E.in3)), s: lt < a ? 0.4 : mix(0.4, 1, ease(lt, a, a + 0.45, E.back)), y: Math.sin(lt * 1.6 + i) * 10 });
      });
      put(chipsTitle, { o: ease(lt, 5.8, 6.1) * (1 - ease(lt, 7.7, 8, E.in3)), s: mix(1.3, 1, ease(lt, 5.8, 6.3, E.expo)), blur: mix(12, 0, ease(lt, 5.8, 6.2)) });
    });
  }

  // 5. 网关拓扑 ——————————————————————————
  {
    const root = $("#s-net");
    const ICON = n => `/repo/site/assets/agents/${n}.svg`;
    const AG = [
      { nm: "Codex", st: "默认模型 · 主力编程", ic: `<img src="${ICON("chatgpt")}">`, color: "#7ba7ff" },
      { nm: "Claude Code", st: "opus · sonnet · haiku", ic: `<img src="${ICON("claude")}">`, color: "#ff8a50" },
      { nm: "Grok", st: "已接入", ic: `<img src="${ICON("grok")}">`, color: "#c9d1ff" },
      { nm: "Gemini CLI", st: "已接入", ic: "G", color: "#9c86ff" },
      { nm: "OpenCode", st: "已接入", ic: "OC", color: "#3ecf8e" },
      { nm: "Zed", st: "已接入", ic: "Z", color: "#ffb347" },
    ];
    const PV = [
      { nm: "ChatGPT 订阅", st: "订阅登录 · 5h 已用 23%", ic: `<img src="${ICON("chatgpt")}">` },
      { nm: "DeepSeek", st: "Chat · 2 个模型", ic: "DS" },
      { nm: "Kimi", st: "Chat · 2 个 Key 轮换", ic: "K" },
      { nm: "智谱 GLM", st: "Chat · 1 个模型", ic: "GLM" },
      { nm: "sub2api 站点", st: "倍率 0.28x", ic: "S2" },
      { nm: "new-api 站点", st: "剩 ¥36.50", ic: "NA" },
    ];
    const ys = [250, 370, 490, 610, 730, 850];
    const AX = 330, PX = 1590, HUB = { l: 780, r: 1140, y: 540 };
    const nodesEl = $(".nodes", root);
    const mkNode = (d, x, y) => { const el = div("node", `<div class="ic">${d.ic}</div><div><div class="nm">${d.nm}</div><div class="st">${d.st}</div></div>`); el.style.left = `${x}px`; el.style.top = `${y}px`; nodesEl.append(el); return el; };
    const agentEls = AG.map((d, i) => mkNode(d, AX, ys[i]));
    const provEls = PV.map((d, i) => mkNode(d, PX, ys[i]));
    const wiresG = $(".wires", root), packetsG = $(".packets", root);
    const NS = "http://www.w3.org/2000/svg";
    const mkPath = d => { const p = document.createElementNS(NS, "path"); p.setAttribute("d", d); p.setAttribute("fill", "none"); p.setAttribute("stroke", "url(#wire)"); p.setAttribute("stroke-width", "2.5"); wiresG.append(p); return p; };
    const aw = ys.map(y => mkPath(`M ${AX + 150} ${y} C ${AX + 330} ${y}, ${HUB.l - 160} ${HUB.y}, ${HUB.l} ${HUB.y}`));
    const pw = ys.map(y => mkPath(`M ${HUB.r} ${HUB.y} C ${HUB.r + 160} ${HUB.y}, ${PX - 330} ${y}, ${PX - 150} ${y}`));
    const lens = { a: aw.map(p => p.getTotalLength()), p: pw.map(p => p.getTotalLength()) };
    const dots = Array.from({ length: 70 }, () => { const c = document.createElementNS(NS, "circle"); c.setAttribute("r", "7"); packetsG.append(c); return c; });
    const booms = Array.from({ length: 12 }, () => { const c = document.createElementNS(NS, "circle"); c.setAttribute("fill", "none"); c.setAttribute("stroke", "#ff5c6c"); c.setAttribute("stroke-width", "4"); packetsG.append(c); return c; });
    // 请求时间表：确定性生成。
    const ROUTE = [[0, 4], [1, 2], [5, 0], [3, 2], [1, 3], [3, 5]];
    const FAIL_AT = 46.5;
    const packets = [];
    let k = 0;
    for (let ts = 42.9; ts < 51.3; ts += 0.15) {
      k++;
      const r = hash(k * 3.1);
      const a = r < 0.3 ? 0 : r < 0.6 ? 1 : 2 + Math.floor(hash(k * 5.7) * 4);
      const p = ROUTE[a][hash(k * 7.3) < 0.55 ? 0 : 1];
      const pk = { a, p, t0: ts, color: AG[a].color };
      if (p === 3 && ts + 0.9 > FAIL_AT) {
        pk.fail = true;
        packets.push({ retry: true, p: 2, t0: ts + 0.45 + 0.4, color: "#ffb347" });
      }
      packets.push(pk);
    }
    const hub = $(".hub", root), nReq = $(".n-req", root), nSwitch = $(".n-switch", root);
    const toast = $(".toast", root), tags = $$(".net-tags span", root), titles = $$(".col-title", root);
    const at = (path, len, p) => path.getPointAtLength(len * clamp(p));
    scene(root, 41.5, 52, (lt, t) => {
      const exit = ease(lt, 10.25, 10.5, E.in3);
      put(hub, { o: ease(lt, 0, 0.3) * (1 - exit), s: mix(0.6, 1, ease(lt, 0, 0.5, E.back)) * (1 + 0.02 * pulse(t)) });
      agentEls.forEach((el, i) => put(el, { o: ease(lt, 0.15 + i * 0.07, 0.4 + i * 0.07) * (1 - exit), x: mix(-80, 0, ease(lt, 0.15 + i * 0.07, 0.6 + i * 0.07, E.expo)) }));
      provEls.forEach((el, i) => put(el, { o: ease(lt, 0.25 + i * 0.07, 0.5 + i * 0.07) * (1 - exit), x: mix(80, 0, ease(lt, 0.25 + i * 0.07, 0.7 + i * 0.07, E.expo)) }));
      titles.forEach(el => put(el, { o: ease(lt, 0.3, 0.6) * (1 - exit) }));
      const draw = ease(lt, 0.4, 1.4, E.io3);
      [...aw, ...pw].forEach((path, i) => {
        const len = i < 6 ? lens.a[i] : lens.p[i - 6];
        path.style.strokeDasharray = `${len}`;
        path.style.strokeDashoffset = `${len * (1 - draw)}`;
        path.style.opacity = 1 - exit;
      });
      // 请求流
      let used = 0, boomUsed = 0, arrived = 0, switched = 0, kimiHot = false;
      for (const pk of packets) {
        const tt = t - pk.t0;
        if (!pk.retry && tt >= 0.45) arrived++;
        if (pk.retry && tt >= 0) switched++;
        if (pk.retry && tt >= 0.45 && tt < 0.8) kimiHot = true;
        let pt = null, color = pk.color, o = 1;
        if (pk.retry) {
          if (tt >= 0 && tt < 0.45) pt = at(pw[pk.p], lens.p[pk.p], tt / 0.45);
        } else if (tt >= 0 && tt < 0.45) pt = at(aw[pk.a], lens.a[pk.a], E.io3(tt / 0.45));
        else if (tt >= 0.45 && tt < 0.9) {
          const p2 = (tt - 0.45) / 0.45;
          if (pk.fail) {
            if (p2 < 0.82) pt = at(pw[pk.p], lens.p[pk.p], p2); else if (boomUsed < booms.length) {
              const bp = lin(p2, 0.82, 1.6);
              const c = at(pw[pk.p], lens.p[pk.p], 0.82);
              const b = booms[boomUsed++];
              b.setAttribute("cx", c.x); b.setAttribute("cy", c.y); b.setAttribute("r", 8 + 40 * E.out3(bp)); b.style.opacity = 1 - bp;
            }
            color = "#ff5c6c";
          } else pt = at(pw[pk.p], lens.p[pk.p], p2);
        } else if (pk.fail && tt >= 0.9 && tt < 1.25 && boomUsed < booms.length) {
          const bp = lin(tt, 0.82 * 0.45 + 0.45, 1.25);
          const c = at(pw[pk.p], lens.p[pk.p], 0.82);
          const b = booms[boomUsed++];
          b.setAttribute("cx", c.x); b.setAttribute("cy", c.y); b.setAttribute("r", 8 + 40 * E.out3(bp)); b.style.opacity = 1 - bp;
        }
        if (pt && used < dots.length) {
          const d = dots[used++];
          d.setAttribute("cx", pt.x.toFixed(1)); d.setAttribute("cy", pt.y.toFixed(1)); d.setAttribute("fill", color); d.style.opacity = o * (1 - exit);
        }
      }
      for (let i = used; i < dots.length; i++) dots[i].style.opacity = 0;
      for (let i = boomUsed; i < booms.length; i++) booms[i].style.opacity = 0;
      nReq.textContent = String(896 + arrived);
      nSwitch.textContent = String(22 + switched);
      const bad = t >= FAIL_AT;
      provEls[3].classList.toggle("bad", bad);
      $(".st", provEls[3]).textContent = bad ? `429 · 被限流 ${Math.max(0, 42 - Math.floor(t - FAIL_AT))}s 后恢复` : PV[3].st;
      provEls[2].classList.toggle("hot", kimiHot);
      put(toast, { o: env(t, 47.0, 51.6, 0.25, 0.3), y: mix(20, 0, ease(t, 47.0, 47.4, E.expo)) });
      tags.forEach((tag, i) => put(tag, { o: ease(lt, 7.6 + i * 0.25, 7.9 + i * 0.25) * (1 - exit), y: mix(24, 0, ease(lt, 7.6 + i * 0.25, 8.1 + i * 0.25, E.expo)) }));
    });
  }

  // 6/8/9. 截图讲解 ——————————————————————
  {
    const root = $("#s-shots");
    const holder = $(".shot-holder", root), marksEl = $(".marks", root), sideEl = $(".side", root);
    const W = 1230;
    const li = items => `<ul>${items.map(x => `<li>${x}</li>`).join("")}</ul>`;
    const routingMain = { x: 338, y: 284, w: 1000, h: 195 };
    const SEG = [
      {
        t0: 52, t1: 56, shot: "providers", step: "02 · 共享供应商", title: "加一次，<br>所有 Agent 都能用",
        side: li(["填 API Key，或登录 ChatGPT / Copilot / SuperGrok 订阅", "自动拉模型列表，按公开价格表填好价格", "中转站自动识别倍率与余额"]),
        cams: () => [[0.3, {}], [0.9, { ...center(R("providers", "chatgpt")), z: 1.38 }], [2.0, { ...center(R("providers", "chatgpt")), z: 1.38 }], [2.5, { ...center(R("providers", "glm")), z: 1.38 }]],
        marks: () => [
          { r: R("providers", "chatgpt"), t0: 1.0, t1: 2.05, tip: "订阅登录进池，剩余额度一目了然" },
          { r: R("providers", "glm"), t0: 2.6, t1: 3.85, tip: "被限流自动避开，恢复后再用", place: "below" },
        ],
      },
      {
        t0: 56, t1: 60, shot: "routing", step: "02 · 路由", title: "一个模型名，<br>三种发法",
        side: `<div class="syntax"><div><code>deepseek-v4-pro</code>池里有它的供应商自动成组，出错换下一家</div><div><code>deepseek/deepseek-v4-pro</code>只发给这一家</div><div><code>group/主力编程</code>走自建的模型组</div></div>`,
        cams: () => [[0.3, {}], [0.9, { ...center(routingMain), z: 1.38 }], [3.8, { ...center(routingMain), z: 1.4 }]],
        marks: () => [
          { r: routingMain, t0: 1.0, t1: 3.85, tip: "依次 · 轮换 · 按权重 · 额度优先 · 速度优先", place: "below" },
          { r: R("routing", "mode"), t0: 1.8, t1: 3.85, tip: "按会话固定，缓存不丢" },
        ],
      },
      {
        t0: 60, t1: 64, shot: "agents", step: "02 · Agent 接入", title: "开关一拨，<br>就接上了",
        side: li(["接入前先存档 Agent 的配置文件", "断开时只还原改过的那几项", "Claude Code 可分别指定 opus / sonnet / haiku"]),
        cams: () => [[0.3, {}], [0.9, { cx: 838, cy: 236, z: 1.38 }]],
        marks: () => [
          { r: R("agents", "codex"), t0: 1.0, t1: 3.85, tip: "Codex：默认用「主力编程」模型组" },
          { r: R("agents", "claude"), t0: 1.6, t1: 3.85, tip: "Claude Code：按档位各选模型", place: "below" },
        ],
      },
      {
        t0: 72.5, t1: 76, shot: "provider-detail", step: "03 · 本地服务里也一样", title: "供应商详情里，<br>按模型设置",
        side: li(["每个模型单独设上下文窗口、价格、思考等级", "可按公开资料一键填", "请求比某家的窗口还长：这家排到最后再试"]),
        cams: () => [[0.3, {}], [0.9, { ...center(R("provider-detail", "card")), z: 1.38 }]],
        marks: () => [{ r: R("provider-detail", "card"), t0: 1.0, t1: 3.35, tip: "展开一家供应商：模型、Key 与更多设置", place: "below" }],
      },
      {
        t0: 77.5, t1: 84, shot: "usage", step: "04 · 统计", title: "花了多少，<br>一眼看清",
        side: `<div class="kpis"><div class="kpi"><small>估算费用</small><b data-count="30.50" data-fmt="¥">0</b></div><div class="kpi"><small>Tokens</small><b data-count="8.8" data-fmt="M">0</b></div><div class="kpi"><small>缓存命中</small><b data-count="62" data-fmt="%">0</b></div><div class="kpi"><small>成功率</small><b data-count="99.1" data-fmt="%1">0</b></div></div>${li(["按天看费用、token、缓存与调用", "按 Agent、供应商、模型拆开看"])}`,
        cams: () => [[0.3, {}], [0.9, { ...center(R("usage", "kpi")), z: 1.4 }], [2.6, { ...center(R("usage", "kpi")), z: 1.4 }], [3.3, { ...center(R("usage", "chart")), z: 1.38 }], [6.4, { ...center(R("usage", "chart")), z: 1.38 }]],
        marks: () => [{ r: R("usage", "kpi"), t0: 1.0, t1: 2.6, tip: "经 Codex++ 的每次调用都记账" }],
      },
      {
        t0: 84, t1: 88, shot: "requests", step: "04 · 请求日志", title: "每条请求，<br>都有据可查",
        side: li(["状态、自动重试、首字耗时", "输入 / 输出 / 缓存 token 与估算费用", "失败的直接显示原因"]),
        cams: () => [[0.3, {}], [0.9, { ...center(R("requests", "limited")), z: 1.38 }]],
        marks: () => [{ r: R("requests", "limited"), t0: 1.0, t1: 3.85, tip: "429 限流 → 自动换下一家", place: "below" }],
      },
      {
        t0: 88, t1: 92, shot: "requests", step: "04 · 托盘快捷面板", title: "不开主窗口，<br>也能看",
        side: li(["点托盘图标就能换模型", "看今日用量和实时请求流", "供应商出错，一眼就知道"]),
        cams: () => [[0, { ...center(R("requests", "limited")), z: 1.38 }], [0.6, {}]],
        marks: () => [],
        tray: true,
      },
    ];
    for (const seg of SEG) {
      seg.win = makeWin(seg.shot, W);
      seg.win.el.style.left = "60px";
      seg.win.el.style.top = `${(1080 - seg.win.h) / 2}px`;
      holder.append(seg.win.el);
      seg.sideEl = div("", `<div class="step-no">${seg.step}</div><h3>${seg.title}</h3>${seg.side}`);
      sideEl.append(seg.sideEl);
      seg.markEls = [0, 1].map(() => { const m = div("mark"), tp = div("tip"); marksEl.append(m, tp); return { m, tp }; });
      if (seg.tray) {
        seg.trayWin = makeWin("tray", 440, { base: [400, 640], title: "托盘" });
        holder.append(seg.trayWin.el);
      }
    }
    const update = (lt0, t) => {
      for (const seg of SEG) {
        const on = t >= seg.t0 && t < seg.t1;
        seg.win.el.style.display = on ? "" : "none";
        seg.sideEl.style.display = on ? "" : "none";
        if (seg.trayWin) seg.trayWin.el.style.display = on ? "" : "none";
        if (!on) { seg.markEls.forEach(({ m, tp }) => { put(m, { o: 0 }); put(tp, { o: 0 }); }); continue; }
        const lt = t - seg.t0, dur = seg.t1 - seg.t0;
        const first = seg === SEG[0] || seg === SEG[3] || seg === SEG[4];
        const enter = ease(lt, 0, 0.5, E.expo);
        const exit = ease(lt, dur - 0.25, dur, E.in3);
        const left = 60, top = (1080 - seg.win.h) / 2;
        const trayPush = seg.tray ? ease(lt, 0.3, 1.0, E.io3) : 0;
        put(seg.win.el, {
          o: (seg.tray ? 1 : enter) * (1 - (seg.t1 === 92 || seg.t1 === 64 || seg.t1 === 76 ? exit : 0)) * (1 - trayPush * 0.55),
          x: seg.tray ? -trayPush * 60 : mix(first ? -200 : 160, 0, enter),
          ry: seg.tray ? trayPush * 12 : mix(first ? 25 : -18, 0, enter),
          s: (1 + lin(lt, 0, dur) * 0.02) * (1 - trayPush * 0.1),
          blur: trayPush * 3,
        });
        setCam(seg.win, camAt(lt, seg.cams()));
        if (seg.trayWin) {
          const tw = seg.trayWin;
          tw.el.style.left = `${780}px`; tw.el.style.top = `${(1080 - tw.h) / 2}px`;
          const te = ease(lt, 0.5, 1.2, E.expo);
          put(tw.el, { o: te * (1 - exit), y: mix(260, 0, te), ry: mix(-35, -6, te), rx: mix(20, 4, te), s: 1 + 0.015 * Math.sin(lt * 2) });
          setCam(tw, { z: 1 });
        }
        const marks = seg.marks();
        seg.markEls.forEach(({ m, tp }, i) => {
          const mk = marks[i];
          if (!mk || !mk.r || lt < mk.t0 || lt > mk.t1) { put(m, { o: 0 }); put(tp, { o: 0 }); return; }
          tp.textContent = mk.tip;
          drawMark(m, tp, mapRect(seg.win, left, top, mk.r), env(lt, mk.t0, mk.t1, 0.25, 0.2), { place: mk.place ?? "above" });
        });
        // 右侧说明
        const sideIn = (d = 0) => ease(lt, 0.25 + d, 0.6 + d);
        const kids = [...seg.sideEl.querySelectorAll(".step-no, h3, li, .kpi, .syntax > div")];
        kids.forEach((kid, i) => put(kid, { o: sideIn(i * 0.12) * (1 - exit), x: mix(50, 0, ease(lt, 0.25 + i * 0.12, 0.75 + i * 0.12, E.expo)) }));
        seg.sideEl.querySelectorAll("[data-count]").forEach((el, i) => {
          const p = ease(lt, 0.5 + i * 0.15, 1.8 + i * 0.15, E.out3);
          const v = Number(el.dataset.count) * p, fmt = el.dataset.fmt;
          el.textContent = fmt === "¥" ? `¥${v.toFixed(2)}` : fmt === "M" ? `${v.toFixed(1)}M` : fmt === "%1" ? `${v.toFixed(1)}%` : `${Math.round(v)}%`;
        });
      }
    };
    scene(root, 52, 64, update);
    scene(root, 72.5, 76, update);
    scene(root, 77.5, 92, update);
  }

  // 7. 每模型窗口 ————————————————————————
  {
    const root = $("#s-window");
    const list = $(".ed-list", root), toml = $(".ed-toml", root), json = $(".ed-json", root), arrow = $(".flow-arrow", root);
    const listCode = $(".code", list), tomlCode = $(".code", toml), jsonCode = $(".code", json);
    const LINES = [["deepseek-v4-pro", "[1M]"], ["your-model", "[200K]"], ["another-model", "[128000]"]];
    const TOML = [`<span class="c"># Codex++ 写入的指针</span>`, `<span class="k">model_catalog_json</span> = <span class="s">"~/.codex/model-catalogs/relay-default.json"</span>`];
    const JSONL = [
      `{ <span class="s">"models"</span>: [`,
      `  { <span class="s">"slug"</span>: <span class="s">"deepseek-v4-pro"</span>, <span class="k">"context_window"</span>: <span class="n">1000000</span> },`,
      `  { <span class="s">"slug"</span>: <span class="s">"your-model"</span>,      <span class="k">"context_window"</span>: <span class="n">200000</span> },`,
      `  { <span class="s">"slug"</span>: <span class="s">"another-model"</span>,   <span class="k">"context_window"</span>: <span class="n">128000</span> }`,
      `] }`,
    ];
    const barsEl = $(".bars", root);
    const BARS = [["deepseek-v4-pro", 1000000, "1M"], ["your-model", 200000, "200K"], ["another-model", 128000, "128K"]].map(([name, v, label]) => {
      const b = div("b", `<div class="lbl">${name}</div><div class="track"><div class="fill"></div></div><div class="val"></div>`);
      barsEl.append(b);
      return { b, fill: $(".fill", b), val: $(".val", b), v, label };
    });
    const note = $(".window-note", root);
    // 逐行打出 HTML：先按纯文本长度截断，再补回高亮（只在整行打完后高亮）。
    const strip = html => html.replace(/<[^>]+>/g, "");
    const typeLines = (lines, a, step, lt) => lines.map((line, i) => {
      const p = lin(lt, a + i * step, a + (i + 0.9) * step);
      if (p <= 0) return "";
      if (p >= 1) return line;
      return typed(strip(line), p).replace(/&/g, "&amp;").replace(/</g, "&lt;");
    }).join("\n");
    scene(root, 65.5, 72.5, lt => {
      const exit = ease(lt, 6.75, 7, E.in3);
      put(list, { o: ease(lt, 0, 0.3) * (1 - exit), x: mix(-200, 0, ease(lt, 0, 0.6, E.expo)), ry: mix(25, 0, ease(lt, 0, 0.6, E.expo)) });
      listCode.innerHTML = LINES.map(([name, sfx], i) => {
        const a = 0.5 + i * 0.75;
        const nameP = lin(lt, a, a + 0.45), sfxP = lin(lt, a + 0.45, a + 0.65);
        if (nameP <= 0) return "";
        const sfxText = typed(sfx, sfxP);
        const glow = sfxP > 0 ? ` style="text-shadow:0 0 ${(20 * Math.exp(-Math.max(0, lt - a - 0.65) / 0.4)).toFixed(1)}px #ff6a2b"` : "";
        return `${typed(name, nameP)}<span class="sfx"${glow}>${sfxText}</span>`;
      }).join("\n") + (Math.floor(lt * 2.4) % 2 === 0 && lt < 3.3 ? "▍" : "");
      put(arrow, { o: ease(lt, 2.9, 3.2) * (1 - exit), sx: ease(lt, 2.9, 3.4, E.expo), s: 1 });
      put(toml, { o: ease(lt, 3.0, 3.3) * (1 - exit), x: mix(200, 0, ease(lt, 3.0, 3.6, E.expo)) });
      tomlCode.innerHTML = typeLines(TOML, 3.2, 0.4, lt);
      put(json, { o: ease(lt, 3.6, 3.9) * (1 - exit), x: mix(200, 0, ease(lt, 3.6, 4.2, E.expo)) });
      jsonCode.innerHTML = typeLines(JSONL, 3.8, 0.28, lt);
      BARS.forEach((bar, i) => {
        const p = ease(lt, 4.8 + i * 0.2, 6.0 + i * 0.2, E.out3);
        put(bar.b, { o: ease(lt, 4.7 + i * 0.2, 4.9 + i * 0.2) * (1 - exit) });
        bar.fill.style.width = `${(p * bar.v / 10000).toFixed(2)}%`;
        bar.val.textContent = p >= 0.999 ? bar.label : `${Math.round(p * bar.v / 1000)}K`;
      });
      put(note, { o: ease(lt, 5.6, 6.0) * (1 - exit), y: mix(20, 0, ease(lt, 5.6, 6.1)) });
    });
  }

  // 10. 功能快剪 + 截图墙 ——————————————————
  {
    const root = $("#s-montage");
    const holder = $(".m-holder", root);
    const ITEMS = [
      ["sessions", "会话", "Codex、Claude Code、Grok 的会话放在一起"],
      ["mcp", "MCP 与 Skills", "一处管理，按 Agent 启用"],
      ["codex-multi", "Codex 多开", "每个供应商一个隔离实例"],
      ["codex-plugins", "Codex 插件", "官方插件市场，可装可卸"],
      ["claude", "Claude Desktop 增强", "完整功能模式：汉化、听写、页面拓展"],
      ["palette", "⌘K", "搜索页面和操作，一步直达"],
    ].map(([shot, name, desc], i) => {
      const win = makeWin(shot, 1300);
      win.el.style.left = `${960 - 650}px`; win.el.style.top = `${40}px`;
      holder.append(win.el);
      return { win, name, desc, t0: 92 + i * 2, cx: shot === "palette" ? 720 : 780 };
    });
    const label = $(".m-label", root), idx = $(".idx", label), nm = $(".name", label), desc = $(".desc", label);
    const wall = $(".wall", root);
    const WALL_SHOTS = ["providers", "routing", "agents", "usage", "requests", "sessions", "ext-featured", "ext-local", "ext-community", "mcp", "codex", "codex-multi", "codex-plugins", "claude", "palette", "provider-detail", "request-routing", "home"];
    const tiles = [];
    for (let r = 0; r < 5; r++) for (let c = 0; c < 7; c++) {
      const name = WALL_SHOTS[(r * 7 + c * 3) % WALL_SHOTS.length];
      const img = document.createElement("img");
      img.src = `/shots/${name}.png`;
      Object.assign(img.style, { position: "absolute", width: "480px", height: "300px", left: `${c * 510 - 300}px`, top: `${r * 330 - 200}px`, borderRadius: "12px", boxShadow: "0 20px 60px rgba(0,0,0,.6)", border: "1px solid rgba(255,255,255,.08)" });
      wall.append(img);
      tiles.push({ img, r, c });
    }
    const w1 = $(".wall-text .w1", root), w2 = $(".wall-text .w2", root);
    scene(root, 92, 108, (lt, t) => {
      ITEMS.forEach((item, i) => {
        const l = t - item.t0;
        const on = l >= 0 && l < 2;
        item.win.el.style.display = on ? "" : "none";
        if (!on) return;
        const e = ease(l, 0, 0.35, E.expo), x = ease(l, 1.75, 2, E.in3);
        put(item.win.el, { o: e * (1 - x), x: mix(700, 0, e) - x * 700, ry: mix(-40, 0, e) + x * 40, z: mix(-300, 0, e) - x * 300, s: 1 + l * 0.02, blur: (1 - e) * 10 + x * 10 });
        // 推近到内容区（截图两侧是侧栏和留白），再慢慢往里推。
        setCam(item.win, { cx: item.cx, cy: 430, z: 1.2 + l * 0.05 });
        idx.textContent = `${String(i + 1).padStart(2, "0")} / 06`;
        nm.textContent = item.name; desc.textContent = item.desc;
        put(label, { o: ease(l, 0.1, 0.3) * (1 - ease(l, 1.8, 2)), x: mix(-40, 0, ease(l, 0.1, 0.5, E.expo)) });
      });
      if (lt >= 12) { put(label, { o: 0 }); }
      const wl = lt - 12;
      wall.style.display = wl >= 0 ? "" : "none";
      if (wl >= 0) {
        const e = ease(wl, 0, 0.8, E.expo);
        put(wall, { o: e * 0.6 * (1 - ease(wl, 3.7, 4, E.in3)), rx: 52, rz: -28, z: mix(-600, -200, lin(wl, 0, 4)), y: mix(300, -120, lin(wl, 0, 4)), x: 120 });
        tiles.forEach(({ img, r, c }) => { img.style.opacity = ease(wl, (r + c) * 0.04, (r + c) * 0.04 + 0.3).toFixed(3); });
      }
      put(w1, { o: ease(wl, 0.3, 0.55) * (1 - ease(wl, 3.7, 4, E.in3)), s: mix(1.25, 1, ease(wl, 0.3, 0.8, E.expo)), blur: mix(14, 0, ease(wl, 0.3, 0.7)) });
      put(w2, { o: ease(wl, 1.3, 1.6) * (1 - ease(wl, 3.7, 4, E.in3)), y: mix(30, 0, ease(wl, 1.3, 1.8, E.expo)) });
    });
  }

  // 11. 片尾 ——————————————————————————————
  {
    const root = $("#s-outro");
    const logo = $(".logo", root), codex = $(".codex", root), pp = $(".pp", root), tag = $(".tag", root);
    const facts = $$(".facts span", root), links = $(".links", root), rings = $$(".ring", root);
    scene(root, 108, 120, (lt, t) => {
      put(logo, { o: ease(lt, 0, 0.12), s: mix(2.6, 1, ease(lt, 0, 0.5, E.expo)) * (1 + 0.03 * pulse(t)), rz: mix(14, 0, ease(lt, 0, 0.6, E.expo)) });
      rings.forEach((ring, i) => { const p = lin(lt, i * 0.15, 1.2 + i * 0.15); put(ring, { o: (1 - p) * (p > 0 ? 0.9 : 0), s: mix(0.3, 3.4, E.out3(p)) }); });
      put(codex, { o: ease(lt, 0.15, 0.4), x: mix(-240, 0, ease(lt, 0.15, 0.7, E.expo)), blur: mix(18, 0, ease(lt, 0.15, 0.6)) });
      put(pp, { o: ease(lt, 0.3, 0.5), x: mix(260, 0, ease(lt, 0.3, 0.85, E.back)), s: mix(1.6, 1, ease(lt, 0.3, 0.85, E.back)) });
      put(tag, { o: ease(lt, 0.9, 1.3), y: mix(30, 0, ease(lt, 0.9, 1.4)) });
      facts.forEach((f, i) => put(f, { o: ease(lt, 1.5 + i * 0.2, 1.8 + i * 0.2), y: mix(24, 0, ease(lt, 1.5 + i * 0.2, 2.0 + i * 0.2, E.expo)) }));
      put(links, { o: ease(lt, 2.4, 2.8), y: mix(24, 0, ease(lt, 2.4, 2.9, E.expo)) });
      put(root, { s: mix(1, 1.05, lin(lt, 0, 12)) });
    }, "flex");
  }

  // —————————— 叠加层：闪白、故障条、HUD、淡出 ——————————
  const CUTS = [[8, 1], [12, 0.45], [20, 0.6], [21.5, 0.25], [32, 0.35], [40, 0.6], [41.5, 0.25], [52, 0.35], [56, 0.18], [60, 0.18], [64, 0.55], [65.5, 0.25], [72.5, 0.25], [76, 0.6], [77.5, 0.25], [84, 0.18], [92, 0.45], [94, 0.15], [96, 0.15], [98, 0.15], [100, 0.15], [102, 0.15], [104, 0.4], [108, 1]];
  const flash = $("#flash"), glitch = $("#glitch"), fade = $("#fade");
  const gbars = Array.from({ length: 9 }, () => { const i = document.createElement("i"); glitch.append(i); return i; });
  const hud = $("#hud"), chap = $(".chap", hud), tc = $(".tc", hud), prog = $(".prog i", hud);
  const CHAP = [[20, 40, "01 · 万物皆拓展"], [40, 64, "02 · 本地服务"], [64, 76, "03 · 每个模型，各有空间"], [76, 92, "04 · 用量与请求"], [92, 108, "05 · 还有更多"]];
  function overlays(t) {
    let f = 0, gl = 0, gi = 0;
    CUTS.forEach(([c, s], i) => {
      if (t >= c) { f = Math.max(f, s * Math.exp(-(t - c) / 0.11)); if (s >= 0.3 && t - c < 0.2) { gl = Math.max(gl, 1 - (t - c) / 0.2); gi = i; } }
    });
    flash.style.opacity = (f * 0.85).toFixed(3);
    gbars.forEach((bar, j) => {
      if (gl <= 0) { bar.style.opacity = 0; return; }
      const seed = gi * 31 + j * 7 + Math.floor(t * 30);
      bar.style.top = `${(hash(seed) * 1080).toFixed(0)}px`;
      bar.style.height = `${(4 + hash(seed + 1) * 46).toFixed(0)}px`;
      bar.style.background = ["rgba(255,106,43,.5)", "rgba(156,134,255,.5)", "rgba(123,167,255,.45)", "rgba(255,255,255,.35)"][j % 4];
      bar.style.transform = `translateX(${((hash(seed + 2) - 0.5) * 240).toFixed(0)}px)`;
      bar.style.opacity = (gl * (0.5 + 0.5 * hash(seed + 3))).toFixed(3);
    });
    const active = scenes.filter(s => t >= s.t0 && t < s.t1);
    for (const s of active) s.el.style.translate = gl > 0 ? `${((hash(Math.floor(t * 30) + 5) - 0.5) * 40 * gl).toFixed(1)}px 0` : "0 0";
    const hudOn = t >= 12 && t < 108;
    hud.style.opacity = hudOn ? (env(t, 12, 108, 0.5, 0.4)).toFixed(3) : 0;
    const c = CHAP.find(([a, b]) => t >= a && t < b);
    chap.textContent = c ? c[2] : "";
    const fr = Math.floor(t * 30);
    tc.textContent = `${String(Math.floor(fr / 1800)).padStart(2, "0")}:${String(Math.floor(fr / 30) % 60).padStart(2, "0")}:${String(fr % 30).padStart(2, "0")}  ·  YOUR AGENTS, UPGRADED.`;
    prog.style.width = `${(t / DURATION * 100).toFixed(2)}%`;
    fade.style.opacity = Math.max(1 - lin(t, 0, 0.4), ease(t, 117.5, 120, E.io3)).toFixed(3);
  }

  function seek(t) {
    t = clamp(t, 0, DURATION - 1e-6);
    drawBg(t);
    const active = new Set(scenes.filter(s => t >= s.t0 && t < s.t1).map(s => s.el));
    for (const s of scenes) if (!active.has(s.el)) s.el.style.display = "none";
    for (const s of scenes) if (t >= s.t0 && t < s.t1) { s.el.style.display = s.display; s.update(t - s.t0, t); }
    overlays(t);
  }

  // —————————— 启动：等区域数据、图片与字体就绪 ——————————
  const ready = (async () => {
    REG = await (await fetch("/shots/regions.json")).json();
    await document.fonts.ready;
    await Promise.all($$("img").map(img => img.decode().catch(() => console.warn("图片加载失败", img.src))));
  })();
  window.__ready = ready;
  window.__seek = seek;
  window.__duration = DURATION;

  const params = new URLSearchParams(location.search);
  if (params.has("render")) { document.body.classList.add("render"); return; }

  // 预览播放器
  const stage = $("#stage");
  const fit = () => { const k = Math.min(innerWidth / 1920, (innerHeight - 44) / 1080); stage.style.transform = `scale(${k})`; };
  addEventListener("resize", fit); fit();
  const audio = new Audio("/promo/music.wav");
  const play = $("#play"), scrub = $("#scrub"), clock = $("#clock");
  let playing = false, t = Number(params.get("t") ?? 0);
  ready.then(() => seek(t));
  play.onclick = () => { playing = !playing; play.textContent = playing ? "❚❚ 暂停" : "▶ 播放"; if (playing) { audio.currentTime = t; audio.play(); } else audio.pause(); };
  scrub.oninput = () => { t = Number(scrub.value); audio.currentTime = t; seek(t); };
  const loop = () => {
    if (playing) { t = audio.currentTime; if (t >= DURATION) { playing = false; } seek(t); scrub.value = t; }
    clock.textContent = t.toFixed(2);
    requestAnimationFrame(loop);
  };
  requestAnimationFrame(loop);
})();
