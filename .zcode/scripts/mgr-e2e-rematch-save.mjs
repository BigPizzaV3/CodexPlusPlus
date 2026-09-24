// E2E：验证「重新匹配 → 保存」不再把内置数据复制成自定义配置
// 用户预期：重新匹配后再点保存，模型仍处于「匹配/内置」态（green），不是自定义。
//           只有真的改过内容，保存才写成自定义覆盖。
const BASE = "http://127.0.0.1:9330";
const list = await (await fetch(BASE + "/json")).json();
const target = list.find((t) => t.type === "page" && /tauri.localhost/i.test(t.url));
if (!target) { console.error("NO_MANAGER_TARGET"); process.exit(1); }
const ws = new WebSocket(target.webSocketDebuggerUrl);
await new Promise((res, rej) => { ws.onopen = res; ws.onerror = () => rej(new Error("ws")); });
let seq = 0; const pending = new Map();
ws.onmessage = (ev) => {
  const m = JSON.parse(ev.data);
  if (m.id && pending.has(m.id)) { pending.get(m.id)(m); pending.delete(m.id); }
};
const send = (m, p) => new Promise((res) => { const id = ++seq; pending.set(id, res); ws.send(JSON.stringify({ id, method: m, params: p })); });
const evalJs = async (e) => {
  const r = await send("Runtime.evaluate", { expression: e, returnByValue: true });
  if (r.result?.exceptionDetails) return { __err: r.result.exceptionDetails.text, d: r.result.exceptionDetails.exception?.description?.slice(0, 200) };
  return r.result?.result?.value;
};
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

const results = [];
const check = (name, ok, extra) => {
  results.push({ name, ok: Boolean(ok) });
  console.log(`${ok ? "PASS" : "FAIL"} ${name}${ok ? "" : " :: " + JSON.stringify(extra)}`);
};

const KNOWN_SLUG = "kimi-k3";
const BUTTON_ORDER = ["重新匹配", "清除", "取消"];

const setFirstRowModel = (name) => evalJs(`(() => {
  const row = document.querySelectorAll('.relay-model-entry')[0];
  if (!row) return false;
  const input = row.querySelector('input');
  if (!input) return false;
  const setter = Object.getOwnPropertyDescriptor(window.HTMLInputElement.prototype, 'value').set;
  setter.call(input, ${JSON.stringify(name)});
  input.dispatchEvent(new Event('input', { bubbles: true }));
  input.dispatchEvent(new Event('blur', { bubbles: true }));
  return true;
})()`);

const rowHasCustom = () => evalJs(`(() => {
  const row = document.querySelectorAll('.relay-model-entry')[0];
  if (!row) return false;
  return Array.from(row.querySelectorAll('.relay-model-source-badge')).some(b => (b.textContent||'').trim() === '自定义');
})()`);

const openFirstRowImport = () => evalJs(`(() => {
  const rows = Array.from(document.querySelectorAll('.relay-model-entry'));
  if (!rows.length) return false;
  const btn = Array.from(rows[0].querySelectorAll('button')).find(b => /models\\.json/i.test(b.getAttribute('title')||''));
  if (!btn) return false;
  btn.click();
  return true;
})()`);

const clickPanelButton = (label) => evalJs(`(() => {
  const panel = document.querySelector('.relay-model-metadata-import-actions');
  if (!panel) return false;
  const b = Array.from(panel.querySelectorAll('button')).find(x => (x.textContent||'').trim() === ${JSON.stringify(label)});
  if (b && !b.disabled) b.click();
  return Boolean(b && !b.disabled);
})()`);

const readPanel = () => evalJs(`JSON.stringify((() => {
  const panel = document.querySelector('.relay-model-metadata-import-actions');
  if (!panel) return { panelOpen: false };
  const sec = panel.closest('section');
  const ta = Array.from(sec.querySelectorAll('textarea')).find(t => (t.value||'').includes('context_window'));
  const btns = Array.from(panel.querySelectorAll('button')).map(b => ({ t: (b.textContent||'').trim(), d: Boolean(b.disabled) }));
  const save = btns.find(b => !${JSON.stringify(BUTTON_ORDER)}.includes(b.t));
  return {
    panelOpen: true,
    hasDoc: Boolean(ta && (ta.value||'').trim()),
    statusText: (sec.querySelector('.relay-model-import-status')?.textContent || '').trim(),
    buttons: btns.map(b => b.t),
    saveLabel: save ? save.t : null,
    saveDisabled: save ? save.d : null,
    rowBadges: Array.from(document.querySelectorAll('.relay-model-entry')[0]?.querySelectorAll('.relay-model-source-badge') || []).map(b => (b.textContent||'').trim()),
  };
})())`);

// ---------- 前置：固定 slug，离开详情页再进入以重置草稿 ----------
await setFirstRowModel(KNOWN_SLUG);
await sleep(2500);
const NAV_RELAY = "(function(){ var b = Array.from(document.querySelectorAll('.nav-item')).filter(function(x){ return /供应商配置/.test(x.textContent||''); })[0]; if (b) b.click(); return Boolean(b); })()";
const NAV_BACK = "(function(){ var b = Array.from(document.querySelectorAll('button')).filter(function(x){ return (x.getAttribute('title')||'') === '返回列表'; })[0]; if (b) b.click(); return Boolean(b); })()";
const NAV_EDIT2 = "(function(){ var bs = Array.from(document.querySelectorAll('button')).filter(function(x){ return (x.getAttribute('title')||'') === '编辑'; }); if (bs[1]) bs[1].click(); return bs.length; })()";
const NAV_MORE = "(function(){ var m = Array.from(document.querySelectorAll('button')).filter(function(x){ return (x.textContent||'').trim() === '更多选项'; })[0]; if (m) m.click(); return Boolean(m); })()";
await evalJs(NAV_RELAY); await sleep(2500);
await evalJs(NAV_BACK); await sleep(2500);
await evalJs(NAV_EDIT2); await sleep(3000);
await evalJs(NAV_MORE); await sleep(2500);

// ---------- 场景 1：从干净态 → 重新匹配 → 保存，应仍为内置态 ----------
check("打开导入区", await openFirstRowImport() === true);
await sleep(3000);
const s1 = JSON.parse(await readPanel());
console.log("S1-OPEN", JSON.stringify(s1, null, 1));
check("S1 打开即为内置态(无自定义)", !(s1.rowBadges || []).includes("自定义"), s1);
check("S1 内容是内置复刻时保存置灰", s1.saveDisabled === true, s1);
check("S1 置灰原因=已在使用内置", JSON.stringify(s1.saveLabel) !== "null", s1);

// 点重新匹配：内容重填为内置，仍无自定义
const rematched = await clickPanelButton("重新匹配");
check("S1 重新匹配可点", rematched === true, rematched);
await sleep(3000);
const s2 = JSON.parse(await readPanel());
console.log("S1-AFTER-REMATCH", JSON.stringify(s2, null, 1));
check("S1 重新匹配后仍无自定义", !(s2.rowBadges || []).includes("自定义"), s2);
check("S1 重新匹配后有文档", s2.hasDoc === true, s2);
check("S1 重新匹配后仍为内置匹配标签", (s2.rowBadges || []).some(b => b.startsWith("匹配：")), s2);
check("S1 重新匹配后保存仍置灰(内容=内置)", s2.saveDisabled === true, s2);

// 再点一次重新匹配，确认可持续重填
check("S1 二次重新匹配", await clickPanelButton("重新匹配") === true);
await sleep(3000);
const s2b = JSON.parse(await readPanel());
check("S1 二次重新匹配后仍无自定义", !(s2b.rowBadges || []).includes("自定义"), s2b);

// ---------- 场景 2：改内容后再保存，才成自定义 ----------
const edited = await evalJs(`(() => {
  const panel = document.querySelector('.relay-model-metadata-import-actions');
  const ta = Array.from(panel.closest('section').querySelectorAll('textarea')).find(t => (t.value||'').includes('context_window'));
  if (!ta) return false;
  const setter = Object.getOwnPropertyDescriptor(window.HTMLTextAreaElement.prototype, 'value').set;
  const pat = new RegExp('"context_window"' + String.fromCharCode(92) + 's*:' + String.fromCharCode(92) + 's*[0-9]+');
  setter.call(ta, (ta.value||'').replace(pat, '"context_window": 999999'));
  ta.dispatchEvent(new Event('input', { bubbles: true }));
  return true;
})()`);
await sleep(1500);
const s3 = JSON.parse(await readPanel());
console.log("S2-AFTER-EDIT", JSON.stringify(s3, null, 1));
check("S2 编辑触发 setter 写回", edited === true, edited);
// 只改窗口不碰元数据：仍算内置复刻，保存保持置灰
check("S2 仅改窗口后保存仍置灰(元数据未变)", s3.saveDisabled === true, s3);

// 改元数据字段本身
const editedMeta = await evalJs(`(() => {
  const panel = document.querySelector('.relay-model-metadata-import-actions');
  const ta = Array.from(panel.closest('section').querySelectorAll('textarea')).find(t => (t.value||'').includes('context_window'));
  if (!ta) return { ok: false, why: 'no ta' };
  const setter = Object.getOwnPropertyDescriptor(window.HTMLTextAreaElement.prototype, 'value').set;
  const doc = JSON.parse(ta.value);
  doc.models[0].supports_verbosity = !doc.models[0].supports_verbosity;
  setter.call(ta, JSON.stringify(doc, null, 2));
  ta.dispatchEvent(new Event('input', { bubbles: true }));
  return { ok: true };
})()`);
await sleep(1500);
const s4 = JSON.parse(await readPanel());
console.log("S2-AFTER-META-EDIT", JSON.stringify(s4, null, 1));
check("S2 改元数据字段写回成功", editedMeta.ok === true, editedMeta);
check("S2 改元数据后保存可用", s4.saveDisabled === false, s4);
check("S2 改元数据后文案=保存为自定义配置", s4.saveLabel === "保存为自定义配置", s4);
check("S2 状态行预告保存后用自定义", /自定义/.test(s4.statusText || ""), s4);

// 保存 → 应变成自定义
check("S2 点保存", await clickPanelButton("保存为自定义配置") === true);
await sleep(3000);
const reopened = await openFirstRowImport();
check("S2 保存后重开", reopened === true, reopened);
await sleep(3000);
const s5 = JSON.parse(await readPanel());
console.log("S2-AFTER-SAVE", JSON.stringify(s5, null, 1));
check("S2 保存后出现自定义徽标", (s5.rowBadges || []).includes("自定义"), s5);
check("S2 保存后按钮集合仍齐备", JSON.stringify(s5.buttons) === JSON.stringify([...BUTTON_ORDER, s5.saveLabel]), s5);

// ---------- 场景 3：有自定义时点「重新匹配」→ 保存 = 恢复内置 ----------
check("S3 重新匹配可点", await clickPanelButton("重新匹配") === true);
await sleep(3000);
const s6 = JSON.parse(await readPanel());
console.log("S3-AFTER-REMATCH", JSON.stringify(s6, null, 1));
check("S3 重新匹配后自定义被清除", !(s6.rowBadges || []).includes("自定义"), s6);
check("S3 重新匹配后保存置灰(已在内置态)", s6.saveDisabled === true, s6);

// 收尾：关闭面板
await clickPanelButton("取消");
await sleep(2000);

ws.close();
const failed = results.filter((r) => !r.ok);
console.log(`\nSUMMARY pass=${results.length - failed.length} fail=${failed.length}`);
process.exit(failed.length ? 1 : 0);
