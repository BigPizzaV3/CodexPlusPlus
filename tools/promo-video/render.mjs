// 渲染宣传片：无头 Chrome 逐帧截图 → Swift/AVFoundation 编码 H.264 → 并入配乐。
// 先跑 capture-shots.mjs 拿到当前界面截图。产物都在 dist/promo/。
// 用法：
//   node tools/promo-video/render.mjs                 出整片 dist/promo/codex-plus-plus-promo.mp4
//   node tools/promo-video/render.mjs --stills 9,23,47  只出几张关键帧 PNG 到 dist/promo/stills/
//   node tools/promo-video/render.mjs --from 40 --to 52 出一段（不带配乐，快速检查）
import { spawn, spawnSync } from "node:child_process";
import { existsSync, mkdirSync, statSync, writeFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { launchChrome, serveStatic } from "./lib/cdp.mjs";

const here = path.dirname(fileURLToPath(import.meta.url));
const repo = path.resolve(here, "../..");
const out = path.join(repo, "dist/promo");
const args = process.argv.slice(2);
const opt = (name, fallback) => (args.includes(name) ? args[args.indexOf(name) + 1] : fallback);
const FPS = Number(opt("--fps", 30));
const W = 1920, H = 1080;

if (!existsSync(path.join(out, "shots/regions.json"))) {
  console.error("缺少界面截图，先运行 node tools/promo-video/capture-shots.mjs");
  process.exit(1);
}

// 编码器：源码比二进制新就重新编译。
const encoder = path.join(out, "encoder");
const encoderSrc = path.join(here, "encoder.swift");
if (!existsSync(encoder) || statSync(encoder).mtimeMs < statSync(encoderSrc).mtimeMs) {
  console.log("编译编码器…");
  const r = spawnSync("swiftc", ["-O", "-swift-version", "5", encoderSrc, "-o", encoder], { stdio: ["ignore", "ignore", "inherit"] });
  if (r.status !== 0) process.exit(1);
}

const server = await serveStatic({ "/": here, "/shots/": path.join(out, "shots"), "/promo/": out, "/repo/": repo });
const page = await launchChrome({ width: W, height: H, scale: 1 });
await page.goto(`${server.url}/index.html?render`);
await page.evaluate("window.__ready");
const duration = await page.evaluate("window.__duration");

async function frame(t, format = "jpeg") {
  await page.evaluate(`window.__seek(${t})`);
  return page.screenshot(format === "png" ? { format: "png" } : { format: "jpeg", quality: 93 });
}

try {
  if (args.includes("--stills")) {
    const dir = path.join(out, "stills");
    mkdirSync(dir, { recursive: true });
    for (const t of opt("--stills").split(",").map(Number)) {
      const file = path.join(dir, `${t.toFixed(2).padStart(6, "0")}.png`);
      writeFileSync(file, await frame(t, "png"));
      console.log(`✓ ${file}`);
    }
  } else {
    const from = Number(opt("--from", 0)), to = Number(opt("--to", duration));
    const partial = from > 0 || to < duration;
    const music = path.join(out, "music.m4a");
    if (!partial && !existsSync(music)) {
      spawnSync(process.execPath, [path.join(here, "music.mjs"), path.join(out, "music.wav"), String(duration)], { stdio: "inherit" });
      spawnSync("afconvert", ["-f", "m4af", "-d", "aac", "-b", "256000", path.join(out, "music.wav"), music], { stdio: "inherit" });
    }
    const silent = path.join(out, partial ? `segment-${from}-${to}.mov` : "video.mov");
    const enc = spawn(encoder, ["encode", silent, String(W), String(H), String(FPS)], { stdio: ["pipe", "inherit", "inherit"] });
    const done = new Promise((resolve, reject) => enc.on("exit", code => (code === 0 ? resolve() : reject(new Error(`编码器退出：${code}`)))));
    const write = buf => new Promise(resolve => { if (enc.stdin.write(buf)) resolve(); else enc.stdin.once("drain", resolve); });
    const total = Math.round((to - from) * FPS);
    const started = Date.now();
    for (let i = 0; i < total; i++) {
      const jpeg = await frame(from + i / FPS);
      const header = Buffer.alloc(4);
      header.writeUInt32BE(jpeg.length);
      await write(header);
      await write(jpeg);
      if (i % 150 === 0 || i === total - 1) {
        const rate = (i + 1) / ((Date.now() - started) / 1000);
        console.log(`帧 ${i + 1}/${total}（${(from + i / FPS).toFixed(1)}s）· ${rate.toFixed(1)} 帧/秒 · 预计还要 ${Math.round((total - i - 1) / rate)} 秒`);
      }
    }
    enc.stdin.end();
    await done;
    if (!partial) {
      const final = path.join(out, "codex-plus-plus-promo.mp4");
      const r = spawnSync(encoder, ["mux", silent, music, final], { stdio: "inherit" });
      if (r.status !== 0) process.exit(1);
    }
  }
} finally {
  server.close();
  await page.close();
}
