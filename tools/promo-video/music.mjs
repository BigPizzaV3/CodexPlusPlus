// 宣传片配乐：程序合成的 120 BPM 合成器电子乐，段落和画面切点对齐（一小节 = 2 秒）。
// 用法：node tools/promo-video/music.mjs <输出.wav> [时长秒，默认 120]
import { writeFileSync } from "node:fs";

const out = process.argv[2] ?? "music.wav";
const DURATION = Number(process.argv[3] ?? 120);
const SR = 44100;
const N = Math.ceil(DURATION * SR);
const L = new Float32Array(N);
const R = new Float32Array(N);
const BEAT = 0.5;
const BAR = 2;

// 段落：drops 是重拍落点；full 段有鼓和贝斯；breakdown 只留铺底和琶音。
const DROPS = [8, 20, 40, 68, 76, 92, 108];
const FULL = [[8, 64], [68, 108]];
const inFull = t => FULL.some(([a, b]) => t >= a && t < b);
const RISERS = [[4, 8], [64, 68], [104, 108]];

// 和弦进行 Am – F – C – G，每个和弦 2 小节。
const CHORDS = [
  { root: 45, notes: [57, 60, 64, 69] },
  { root: 41, notes: [53, 57, 60, 65] },
  { root: 48, notes: [55, 60, 64, 67] },
  { root: 43, notes: [55, 59, 62, 67] },
];
const chordAt = t => CHORDS[Math.floor(t / (BAR * 2)) % CHORDS.length];
const hz = midi => 440 * 2 ** ((midi - 69) / 12);

let seed = 7;
const rand = () => { seed = (seed * 1664525 + 1013904223) >>> 0; return seed / 4294967296 * 2 - 1; };

function add(i, l, r = l) { if (i >= 0 && i < N) { L[i] += l; R[i] += r; } }

// 侧链：每拍底鼓把铺底和贝斯压下去，做出「泵」的感觉。
function duck(t) {
  if (!inFull(t)) return 1;
  const since = t % BEAT;
  return 1 - 0.65 * Math.exp(-since / 0.09);
}

function kick(t0, gain = 1) {
  const len = 0.45 * SR;
  let phase = 0;
  for (let n = 0; n < len; n++) {
    const t = n / SR;
    const f = 45 + 110 * Math.exp(-t / 0.045);
    phase += 2 * Math.PI * f / SR;
    const env = Math.exp(-t / 0.18);
    const click = n < 120 ? rand() * 0.3 * (1 - n / 120) : 0;
    const s = (Math.sin(phase) * env + click) * 0.9 * gain;
    add(Math.round(t0 * SR) + n, s);
  }
}

function clap(t0, gain = 1) {
  const len = 0.25 * SR;
  let lp = 0, lp2 = 0;
  for (let n = 0; n < len; n++) {
    const t = n / SR;
    const x = rand();
    lp += 0.35 * (x - lp);
    lp2 += 0.08 * (lp - lp2);
    const band = lp - lp2;
    const burst = [0, 0.011, 0.022].reduce((sum, d) => sum + (t >= d ? Math.exp(-(t - d) / (d === 0.022 ? 0.07 : 0.008)) : 0), 0);
    const s = band * burst * 0.55 * gain;
    add(Math.round(t0 * SR) + n, s * 0.9, s * 1.1);
  }
}

function hat(t0, open = false, gain = 1) {
  const len = (open ? 0.16 : 0.05) * SR;
  let lp = 0;
  for (let n = 0; n < len; n++) {
    const x = rand();
    lp += 0.6 * (x - lp);
    const hp = x - lp;
    const s = hp * Math.exp(-(n / SR) / (open ? 0.05 : 0.014)) * 0.22 * gain;
    add(Math.round(t0 * SR) + n, s * 0.8, s * 1.2);
  }
}

// 带低通的锯齿波音符；cutoff 可随时间变化。
function saw(t0, dur, freq, { gain = 0.2, attack = 0.005, release = 0.08, cutoff = 1200, cutoffEnv = 0, cutoffDecay = 0.1, detune = 0, pan = 0, ducked = true } = {}) {
  const start = Math.round(t0 * SR);
  const len = Math.round((dur + release) * SR);
  const voices = detune ? [-detune, 0, detune] : [0];
  const phases = voices.map(() => rand() * 0.5 + 0.5);
  let y1 = 0, y2 = 0;
  for (let n = 0; n < len; n++) {
    const t = n / SR;
    let x = 0;
    voices.forEach((cents, v) => {
      phases[v] += freq * 2 ** (cents / 1200) / SR;
      phases[v] -= Math.floor(phases[v]);
      x += phases[v] * 2 - 1;
    });
    x /= voices.length;
    const fc = Math.min(16000, cutoff + cutoffEnv * Math.exp(-t / cutoffDecay));
    const a = 1 - Math.exp(-2 * Math.PI * fc / SR);
    y1 += a * (x - y1);
    y2 += a * (y1 - y2);
    let env = t < attack ? t / attack : 1;
    if (t > dur) env *= Math.max(0, 1 - (t - dur) / release);
    const s = y2 * env * gain * (ducked ? duck(t0 + t) : 1);
    add(start + n, s * (1 - pan) , s * (1 + pan));
  }
}

function impact(t0) {
  const len = 2.2 * SR;
  let phase = 0, lp = 0;
  for (let n = 0; n < len; n++) {
    const t = n / SR;
    phase += 2 * Math.PI * (38 + 60 * Math.exp(-t / 0.08)) / SR;
    lp += 0.05 * (rand() - lp);
    const s = Math.sin(phase) * Math.exp(-t / 0.6) * 0.75 + lp * Math.exp(-t / 0.4) * 1.6;
    add(Math.round(t0 * SR) + n, s, s);
  }
}

function riser(a, b) {
  let lp = 0;
  for (let i = Math.round(a * SR); i < Math.round(b * SR); i++) {
    const p = (i / SR - a) / (b - a);
    const fc = 300 + 9000 * p * p;
    lp += (1 - Math.exp(-2 * Math.PI * fc / SR)) * (rand() - lp);
    const s = lp * p * p * 0.35;
    add(i, s * (1 - 0.3 * Math.sin(p * 40)), s * (1 + 0.3 * Math.sin(p * 40)));
  }
}

// —— 编排 ——
for (let bar = 0; bar * BAR < DURATION; bar++) {
  const t = bar * BAR;
  const chord = chordAt(t);
  // 铺底：每两小节一个和弦，慢起音，片尾段淡出。
  if (bar % 2 === 0) {
    const fade = t >= 108 ? Math.max(0, 1 - (t - 108) / 12) : 1;
    for (const note of chord.notes) saw(t, BAR * 2, hz(note), { gain: 0.05 * fade, attack: 0.6, release: 0.8, cutoff: inFull(t) ? 1800 : 900, detune: 14, pan: (note % 3 - 1) * 0.4 });
  }
  for (let beat = 0; beat < 4; beat++) {
    const bt = t + beat * BEAT;
    if (inFull(bt)) {
      kick(bt);
      if (beat === 1 || beat === 3) clap(bt);
      hat(bt + BEAT / 2, beat === 3);
      hat(bt, false, 0.5);
      // 贝斯：八分音符，根音与高八度交替。
      for (let e = 0; e < 2; e++) {
        const midi = chord.root - 12 + (e === 1 ? 12 : 0);
        saw(bt + e * BEAT / 2, BEAT / 2 - 0.02, hz(midi), { gain: 0.22, cutoff: 220, cutoffEnv: 900, cutoffDecay: 0.06, release: 0.02 });
      }
    }
    // 琶音：十六分音符，入场 4 秒后开始，副歌更亮。
    if (bt >= 4 && bt < 112) {
      for (let s = 0; s < 4; s++) {
        const step = beat * 4 + s;
        const note = chord.notes[[0, 1, 2, 3, 2, 1, 3, 2][step % 8]] + 12;
        const at = bt + s * BEAT / 4;
        const bright = inFull(at) ? 1 : 0.55;
        saw(at, 0.09, hz(note), { gain: 0.06 * bright, cutoff: 600, cutoffEnv: 3500 * bright, cutoffDecay: 0.05, release: 0.06, pan: s % 2 ? 0.5 : -0.5, ducked: false });
      }
    }
  }
}
for (const t of DROPS) impact(t);
for (const [a, b] of RISERS) riser(a, b);

// 琶音来点乒乓延迟：简单做法是整体加一份 3/16 拍延后的高频副本。
const delay = Math.round(BEAT * 0.75 * SR);
for (let i = N - 1; i >= delay; i--) {
  L[i] += R[i - delay] * 0.18;
  R[i] += L[i - delay] * 0.18;
}

// 母带：软限幅、归一化、首尾淡入淡出。
let peak = 0;
for (let i = 0; i < N; i++) {
  L[i] = Math.tanh(L[i] * 1.1);
  R[i] = Math.tanh(R[i] * 1.1);
  peak = Math.max(peak, Math.abs(L[i]), Math.abs(R[i]));
}
const norm = 0.89 / peak;
const buffer = Buffer.alloc(44 + N * 4);
buffer.write("RIFF", 0); buffer.writeUInt32LE(36 + N * 4, 4); buffer.write("WAVE", 8);
buffer.write("fmt ", 12); buffer.writeUInt32LE(16, 16); buffer.writeUInt16LE(1, 20); buffer.writeUInt16LE(2, 22);
buffer.writeUInt32LE(SR, 24); buffer.writeUInt32LE(SR * 4, 28); buffer.writeUInt16LE(4, 32); buffer.writeUInt16LE(16, 34);
buffer.write("data", 36); buffer.writeUInt32LE(N * 4, 40);
for (let i = 0; i < N; i++) {
  const t = i / SR;
  const fade = Math.min(1, t / 0.05, (DURATION - t) / 3);
  buffer.writeInt16LE(Math.round(Math.max(-1, Math.min(1, L[i] * norm * fade)) * 32767), 44 + i * 4);
  buffer.writeInt16LE(Math.round(Math.max(-1, Math.min(1, R[i] * norm * fade)) * 32767), 46 + i * 4);
}
writeFileSync(out, buffer);
console.log(`配乐已写入 ${out}（${DURATION}s）`);
