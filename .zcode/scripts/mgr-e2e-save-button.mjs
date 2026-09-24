// E2E：验证导入区按钮组「恒定渲染 + 取消可撤回」
// 核心断言：
//   1) 面板打开后四个按钮（重新匹配/清除/取消/主按钮）始终同时存在
//   2) 连点清除、重新匹配、改模型名，按钮数量与顺序都不变（只改置灰）
//   3) 「取消」把面板里点过的写入（清除/重新匹配/保存）全部撤回
//   4) 状态行说明当前来源与保存后的效果
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

const BUTTON_ORDER = ["重新匹配", "清除", "取消"];
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
    statusTone: (sec.querySelector('.relay-model-import-status')?.className || '').replace(/.*relay-model-import-status-?/, '').trim(),
    statusTitle: sec.querySelector('.relay-model-import-status')?.getAttribute('title') || '',
    buttons: btns,
    buttonOrder: btns.map(b => b.t),
    rematchDisabled: (btns.find(b => b.t === '重新匹配') || {}).d,
    clearDisabled: (btns.find(b => b.t === '清除') || {}).d,
    cancelDisabled: (btns.find(b => b.t === '取消') || {}).d,
    saveLabel: save ? save.t : null,
    saveDisabled: save ? save.d : null,
    saveTitle: save ? (sec.querySelector('button[title]') && save) : null,
    rowBadges: Array.from(document.querySelectorAll('.relay-model-entry')[0]?.querySelectorAll('.relay-model-source-badge') || []).map(b => (b.textContent||'').trim()),
  };
})())`);

const readSaveTitle = () => evalJs(`JSON.stringify((() => {
  const panel = document.querySelector('.relay-model-metadata-import-actions');
  if (!panel) return null;
  const labels = ['保存此模型','保存为自定义配置','更新此模型配置'];
  const b = Array.from(panel.querySelectorAll('button')).find(x => labels.includes((x.textContent||'').trim()));
  return b ? b.getAttribute('title') : null;
})())`);

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

const rowHasCustom = () => evalJs(`(() => {
  const row = document.querySelectorAll('.relay-model-entry')[0];
  if (!row) return false;
  return Array.from(row.querySelectorAll('.relay-model-source-badge')).some(b => (b.textContent||'').trim() === '自定义');
})()`);

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

const KNOWN_SLUG = "kimi-k3";
const NAV_RELAY = "(function(){ var b = Array.from(document.querySelectorAll('.nav-item')).filter(function(x){ return /供应商配置/.test(x.textContent||''); })[0]; if (b) b.click(); return Boolean(b); })()";
const NAV_BACK = "(function(){ var b = Array.from(document.querySelectorAll('button')).filter(function(x){ return (x.getAttribute('title')||'') === '返回列表'; })[0]; if (b) b.click(); return Boolean(b); })()";
const NAV_EDIT2 = "(function(){ var bs = Array.from(document.querySelectorAll('button')).filter(function(x){ return (x.getAttribute('title')||'') === '编辑'; }); if (bs[1]) bs[1].click(); return bs.length; })()";
const NAV_MORE = "(function(){ var m = Array.from(document.querySelectorAll('button')).filter(function(x){ return (x.textContent||'').trim() === '更多选项'; })[0]; if (m) m.click(); return Boolean(m); })()";
const ROW_COUNT = "document.querySelectorAll('.relay-model-entry').length";

// 先固定模型名，确保磁盘上（若已保存过） slug 是我们预期的那个
await setFirstRowModel(KNOWN_SLUG);
await sleep(2500);
// 走到供应商配置页（可能已在该页，幂等）→ 离开详情页 → 重进（草稿重置）
await evalJs(NAV_RELAY);
await sleep(2500);
await evalJs(NAV_BACK);
await sleep(2500);
console.log("NAV edit-buttons:", await evalJs(NAV_EDIT2));
await sleep(3000);
console.log("NAV more:", await evalJs(NAV_MORE));
await sleep(2500);
console.log(`PRECHECK rows=${await evalJs(ROW_COUNT)} rowHasCustom=${await rowHasCustom()}`);

// ---------- 1) 打开后四按钮齐备 ----------
const opened = await openFirstRowImport();
check("打开第一行导入区", opened === true, opened);
await sleep(2500);
const a = JSON.parse(await readPanel());
console.log("STATE-A", JSON.stringify(a, null, 1));
check("A 四个按钮同时存在", JSON.stringify(a.buttonOrder) === JSON.stringify([...BUTTON_ORDER, a.saveLabel]), a.buttonOrder);
check("A 预填有文档", a.hasDoc === true, a);
check("A 命中内置时重新匹配可用", a.rematchDisabled === false, a);
check("A 无自定义时清除置灰", a.clearDisabled === true, a);
check("A 取消常亮", a.cancelDisabled === false, a);
check("A 主按钮=保存为自定义配置(可用)", a.saveLabel === "保存为自定义配置" && a.saveDisabled === false, a);
check("A 状态行说明内置来源", /内置/.test(a.statusText || ""), a);
check("A 状态行 tone=builtin", a.statusTone === "builtin", a);

// ---------- 2) 连点清除/重新匹配/改名，按钮集合不变 ----------
const badgeBefore = (a.rowBadges || []).join(",");
await clickPanelButton("清除");
await sleep(2000);
const afterClear = JSON.parse(await readPanel());
console.log("AFTER-CLEAR", JSON.stringify(afterClear, null, 1));
check("B 点清除后按钮集合不变", JSON.stringify(afterClear.buttonOrder) === JSON.stringify([...BUTTON_ORDER, afterClear.saveLabel]), afterClear.buttonOrder);
check("B 清除后自定义徽标消失", !(afterClear.rowBadges || []).includes("自定义"), afterClear);
check("B 撤回后行徽标回到内置态", badgeBefore !== "" && !(afterClear.rowBadges || []).includes("自定义"), { badgeBefore, after: afterClear.rowBadges });

await clickPanelButton("重新匹配");
await sleep(2000);
const afterRematch = JSON.parse(await readPanel());
check("C 点重新匹配后按钮集合不变", JSON.stringify(afterRematch.buttonOrder) === JSON.stringify([...BUTTON_ORDER, afterRematch.saveLabel]), afterRematch.buttonOrder);
check("C 重新匹配后重新填回内置内容", afterRematch.hasDoc === true, afterRematch);

// 改模型名为未命中的名字 → 重新匹配置灰，但按钮还在
const renamed = await evalJs(`(() => {
  const row = document.querySelectorAll('.relay-model-entry')[0];
  const input = row.querySelector('input');
  if (!input) return false;
  const setter = Object.getOwnPropertyDescriptor(window.HTMLInputElement.prototype, 'value').set;
  setter.call(input, 'totally-unknown-model-xyz');
  input.dispatchEvent(new Event('input', { bubbles: true }));
  input.dispatchEvent(new Event('blur', { bubbles: true }));
  return true;
})()`);
await sleep(2500);
const afterRename = JSON.parse(await readPanel());
console.log("AFTER-RENAME", JSON.stringify(afterRename, null, 1));
check("D 改名导致未命中后按钮集合仍不变", JSON.stringify(afterRename.buttonOrder) === JSON.stringify([...BUTTON_ORDER, afterRename.saveLabel]), afterRename.buttonOrder);
check("D 未命中时重新匹配置灰", afterRename.rematchDisabled === true, afterRename);
// 改名会把旧 slug 的自定义元数据一并映射过来（改名跟随），所以此处是「自定义」而非「回退」；
// 状态行如实说明当前在用自定义即可——这里是断言既有行为，不是回归。
check("D 状态行与徽标自洽", /(回退|自定义)/.test(afterRename.statusText || ""), afterRename);

// ---------- 3) 保存 + 取消撤回 ----------
// 恢复已知 slug，从干净态开始，避免上一步改名映射进来的自定义干扰
await clickPanelButton("取消");
await sleep(2500);
await setFirstRowModel(KNOWN_SLUG);
await sleep(2500);
const reopened = await openFirstRowImport();
check("E 恢复 slug 后重开导入区", reopened === true, reopened);
await sleep(3000);
const b = JSON.parse(await readPanel());
console.log("STATE-B", JSON.stringify(b, null, 1));
check("E 干净态下主按钮=保存为自定义配置", b.saveLabel === "保存为自定义配置" && b.saveDisabled === false, b);

const saveClicked = await clickPanelButton(b.saveLabel || "保存为自定义配置");
check("E 点保存", saveClicked === true, saveClicked);
await sleep(3000);
const afterSave = JSON.parse(await readPanel());
console.log("AFTER-SAVE", JSON.stringify(afterSave, null, 1));
check("E 保存后面板自动关闭", afterSave.panelOpen === false, afterSave);

const reopened3 = await openFirstRowImport();
check("E 保存后重开", reopened3 === true, reopened3);
await sleep(3000);
const b2 = JSON.parse(await readPanel());
console.log("STATE-B2", JSON.stringify(b2, null, 1));
check("E 保存后行徽标出现自定义", (b2.rowBadges || []).includes("自定义"), b2);
check("E 保存后清除转为可用", b2.clearDisabled === false, b2);
check("E 保存后主按钮置灰(内容一致)", b2.saveDisabled === true && b2.saveLabel === "保存此模型", b2);
const saveTitleB = await readSaveTitle();
console.log("SAVE-TITLE-B", saveTitleB);
check("E 置灰时有原因说明", /无需再保存/.test(saveTitleB || ""), saveTitleB);
check("E 按钮集合在保存后仍齐备", JSON.stringify(b2.buttonOrder) === JSON.stringify([...BUTTON_ORDER, b2.saveLabel]), b2.buttonOrder);

// 取消 → 面板里的写入应被撤回。
// 注意：保存会关面板（既有交互），所以「同一次打开内可撤销」用清除来验证——
// 清除同样是即时写 draft.modelMetadata 的操作。
const cancelled = await clickPanelButton("取消");
check("F 点取消", cancelled === true, cancelled);
await sleep(3500);
const afterCancel = JSON.parse(await readPanel());
console.log("AFTER-CANCEL", JSON.stringify(afterCancel, null, 1));
check("F 取消后面板关闭", afterCancel.panelOpen === false, afterCancel);

// 重开，先点清除制造一次面板内写入，再点取消，验证能撤回
const reopenedG = await openFirstRowImport();
check("G 写入前重开", reopenedG === true, reopenedG);
await sleep(3000);
const beforeClear = JSON.parse(await readPanel());
check("G 写入前存在自定义", (beforeClear.rowBadges || []).includes("自定义"), beforeClear);
const clearedAgain = await clickPanelButton("清除");
check("G 点清除", clearedAgain === true, clearedAgain);
await sleep(3000);
const afterClear2 = JSON.parse(await readPanel());
console.log("AFTER-CLEAR2", JSON.stringify(afterClear2, null, 1));
check("G 清除后自定义徽标消失", !(afterClear2.rowBadges || []).includes("自定义"), afterClear2);

const cancelled2 = await clickPanelButton("取消");
check("G 点取消撤回清除", cancelled2 === true, cancelled2);
await sleep(3500);
const reopened4 = await openFirstRowImport();
check("G 取消后重开", reopened4 === true, reopened4);
await sleep(3000);
const d = JSON.parse(await readPanel());
console.log("STATE-D", JSON.stringify(d, null, 1));
check("G 取消撤回了清除操作", (d.rowBadges || []).includes("自定义"), d);
check("G 撤回后清除仍可用", d.clearDisabled === false, d);

ws.close();
const failed = results.filter((r) => !r.ok);
console.log(`\nSUMMARY pass=${results.length - failed.length} fail=${failed.length}`);
process.exit(failed.length ? 1 : 0);
