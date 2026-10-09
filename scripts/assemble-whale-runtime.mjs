import { readFileSync, writeFileSync } from "node:fs";
import { createHash } from "node:crypto";

const root = new URL("../assets/inject/upstream/whale-widget/", import.meta.url);
const raw = readFileSync(new URL("full-widget.js", root), "utf8");
const expectedHash = "391806b8d4fd7c77a1711e02f58360e641f97b97bfb9085049031365007301b0";
if (createHash("sha256").update(raw).digest("hex") !== expectedHash) throw new Error("上游鲸鱼源码已变化，请核对移植补丁与来源后更新校验值。");
let source = raw;
function patch(before, after) {
  if (source.split(before).length !== 2) throw new Error(`鲸鱼补丁锚点不唯一：${before.slice(0, 80)}`);
  source = source.replace(before, after);
}
patch("if (!dshwIsChatRoot(document.getElementById('root'))) {", "if (false) { // Codex++ 已由 enhancement 生命周期判断挂载时机");
patch("if (dshwIsChatRoot(document.getElementById('root'))) { dshwStartOnce(); return true }", "if (window.__codexPlusWhaleHost.active()) { dshwStartOnce(); return true }");
patch("window.__dshWhaleInit = true", "window.__dshWhaleInit = true\nvar codexWhaleHost = window.__codexPlusWhaleHost");
// Codex 的表单 reset 会隐藏原生 checkbox / range；仅隔离挂件自己的控件。
const controlStyles = `
html input[data-codex-plus-ext="whale-widget"][type="checkbox"]{appearance:auto!important;-webkit-appearance:checkbox!important;display:inline-block!important;position:static!important;width:16px!important;min-width:16px!important;height:16px!important;min-height:16px!important;margin:0 7px 0 0!important;padding:0!important;opacity:1!important;visibility:visible!important;clip:auto!important;clip-path:none!important;accent-color:#203170!important;color-scheme:light!important;cursor:pointer!important}
html input[data-codex-plus-ext="whale-widget"][type="range"]{appearance:none!important;-webkit-appearance:none!important;display:inline-block!important;min-width:0!important;height:24px!important;min-height:24px!important;padding:0!important;border:0!important;box-shadow:none!important;background:transparent!important;color-scheme:light!important;cursor:pointer!important}
html input[data-codex-plus-ext="whale-widget"][type="range"]::-webkit-slider-runnable-track{height:6px!important;border:0!important;border-radius:4px!important;background:#cbd1e1!important}
html input[data-codex-plus-ext="whale-widget"][type="range"]::-webkit-slider-thumb{appearance:none!important;-webkit-appearance:none!important;width:16px!important;height:16px!important;margin-top:-5px!important;border:0!important;border-radius:50%!important;background:#203170!important;box-shadow:none!important}
html input[data-codex-plus-ext="whale-widget"][type="range"]:disabled{opacity:.4!important;cursor:default!important}
`;
patch("styleEl.textContent = css", "styleEl.textContent = css + " + JSON.stringify(controlStyles));
patch("function playPreview(fn) {\n      try { dshwvPreviewOn() }", "function playPreview(fn) {\n      if (soundOn === false) { dshwvToast('声音总开关已关闭，请勾选“声音总开关与按压音效”后试听。'); return }\n      try { dshwvPreviewOn() }");
patch("sndEntry('按压音效'", "sndEntry('声音总开关与按压音效'");
patch("ck.checked = !!opts.checked", "ck.checked = !!opts.checked\n    ck.setAttribute('aria-label', labelText)");
patch("try { dshwInit() } catch (err) {}", "try { dshwInit() } catch (err) { window.__codexPlusWhaleHost.fail(err) }");
patch("function dshwBodyAppend(el) {\n  try {\n    if (!el) return el", "function dshwBodyAppend(el) {\n  try {\n    if (!el || !codexWhaleHost.active()) return el");
patch("    document.body.appendChild(el)\n    if (dshwBodyNodes.indexOf(el)", "    codexWhaleHost.attach(el)\n    document.body.appendChild(el)\n    if (dshwBodyNodes.indexOf(el)");
patch("var i = dshwBodyNodes.indexOf(el)\n    if (i >= 0)", "codexWhaleHost.detach(el)\n    var i = dshwBodyNodes.indexOf(el)\n    if (i >= 0)");
patch("function apiCanAdjustBalance(model) {\n  return !!(model && model.id === 'deepseek' && model.builtin === true &&\n    model.provider === 'deepseek' && model.canAdjustBalance === true)\n}", "function apiCanAdjustBalance(model) { return !!(model && model.canAdjustBalance === true) }");
patch("function bubbleParseDefaultItems() {", "BUBBLE_DEFAULT_ITEMS = codexWhaleHost.defaultBubbleItems(BUBBLE_DEFAULT_ITEMS)\nfunction bubbleParseDefaultItems() {");
patch("function apiModelTodayText(modelId) {", "function apiModelTodayText(modelId) {\n  if (modelId === 'codex') return codexWhaleHost.tokens('today') + ' tokens'");
patch("function bubbleContentTokenMap(m) {\n  m = m || {}\n  var v = ''\n  var map = {}", "function bubbleContentTokenMap(m) {\n  m = m || {}\n  var v = ''\n  var map = {}\n  for (var period of ['today','week','month','all']) map['tokens_' + period] = codexWhaleHost.tokens(period)");
patch("function bubbleTplHelpItems(m) {\n  m = m || {}\n  var arr = []\n  function add(k, d) { arr.push({ k: '{' + k + '}', d: d }) }", "function bubbleTplHelpItems(m) {\n  m = m || {}\n  var arr = []\n  function add(k, d) { arr.push({ k: '{' + k + '}', d: d }) }\n  if (m.modelId === 'codex') for (var p of ['today','week','month','all']) add('tokens_' + p, 'Codex ' + p + ' token 用量')");
patch("  for (var i = 0; i < defs.length; i++) {", "  for (var cp of [['today','今日'],['week','近7天'],['month','本月'],['all','累计']]) defs.push({ key: 'codex-' + cp[0], label: 'Codex ' + cp[1] + ' token', cb: (function(p,l) { return function() { bubbleModuleAdd({type:'today',modelId:'codex',size:8,tpl:l + ' {tokens_' + p + '} tokens'}) } })(cp[0],cp[1]) })\n  for (var i = 0; i < defs.length; i++) {");
patch("function bubblePaletteModule(key) {", "function bubblePaletteModule(key) {\n  if (String(key).indexOf('codex-') === 0) return {type:'today',modelId:'codex',size:8,tpl:'{tokens_' + String(key).slice(6) + '} tokens'}");
patch("var keyRefInp = apiTextInput(isNew ? '' : m.keyRef, '凭据名，例如 OPENROUTER_API_KEY')", "var keyRefInp = codexWhaleHost.profileSelect(isNew ? '' : m.keyRef)");
patch("keyRefInp.title = '写入 DSH 官方凭据（.credentials.yaml）时使用的名字'", "keyRefInp.title = '复用 Codex++ 已保存的供应商配置，密钥只在本机后端使用'");
patch("card.appendChild(apiPanelRow('凭据名', keyRefInp))", "card.appendChild(apiPanelRow('供应商配置', keyRefInp))");
patch("if (!isNew) tplSel.disabled = true", "if (!isNew) tplSel.disabled = true\n    if (m && m.id === 'codex') tplSel.value = 'codex'");
patch("if (!isNew) {\n      btns.appendChild(apiBtn('删除模型'", "if (!isNew && m.id !== 'codex') {\n      btns.appendChild(apiBtn('删除模型'");
patch("keyInp.type = 'password'", "keyInp.type = 'password'\n    keyInp.disabled = true");
patch("keyInp.placeholder = isNew ? '粘贴 API key（保存时写入 DSH 凭据）' : '留空＝不改动现有密钥'", "keyInp.placeholder = '在 Codex++ 供应商配置中管理密钥'");
patch("var delKeyRow = apiPanelRow('', delKey)", "delKey.disabled = true\n    delKey.title = '请在 Codex++ 供应商配置中管理密钥'\n    var delKeyRow = apiPanelRow('', delKey)");
patch("keyRefInp.value = t.keyRef || ''", "keyRefInp.value = keyRefInp.value || (codexWhaleHost.state.relayProfiles[0] || {}).id || ''");
patch("if (modelId === 'deepseek') usageSet[key] = next", "if (modelId === 'current-provider') usageSet[key] = next");
patch("if (m.builtin) pTxt += ' ← 内置模型不生效（始终用内置峰谷价）'", "if (m.builtin && m.provider === 'deepseek') pTxt += ' ← 使用该供应商内置峰谷价'");
patch("} else if (m.builtin) {\n      // 内置 DeepSeek", "} else if (m.builtin && m.provider === 'deepseek') {\n      // 内置 DeepSeek");
patch("baseInp = apiTextInput(isNew ? '' : m.baseUrl, '例如 https://my-gateway.example.com')", "baseInp = apiTextInput(isNew ? '' : m.baseUrl, '复用所选供应商的 Base URL')\n    baseInp.readOnly = true");
patch("var soundOn = true", "var soundOn = codexWhaleHost.initialSound()");
patch("function fmt(balance, currency) {\n  var num = Number(balance)", "function fmt(balance, currency) {\n  if (balance === null || balance === undefined || balance === '') return '—'\n  var num = Number(balance)");
patch("function apiFmtMoney(v, cur) {\n  var n = Number(v)", "function apiFmtMoney(v, cur) {\n  if (v === null || v === undefined || v === '') return '—'\n  var n = Number(v)");
patch("var used = isAuto ? Math.max(0, Number(q.autoUsed) || 0) : Math.max(0, Number(q.used) || 0)", "var rawUsed = isAuto ? q.autoUsed : q.used\n  var used = rawUsed == null || rawUsed === '' ? null : Math.max(0, Number(rawUsed) || 0)");
patch("var left = Math.max(0, total - used)", "var left = used == null ? null : Math.max(0, total - used)");
patch("pct: total > 0 ? Math.min(100, used / total * 100) : 0", "pct: used == null ? null : (total > 0 ? Math.min(100, used / total * 100) : 0)");
patch("function apiFmtQuotaNum(n) {\n  n = Number(n) || 0", "function apiFmtQuotaNum(n) {\n  if (n === null || n === undefined || n === '') return '—'\n  n = Number(n) || 0");
patch("return i.pct.toFixed(1).replace(/\\.0$/, '') + '%'", "return i.pct == null ? '—' : i.pct.toFixed(1).replace(/\\.0$/, '') + '%'");
patch("if (!i) return '已开启（未填总量）'", "if (!i) return '已开启（未填总量）'\n  if (i.used == null) return '统计未完成 · 用量未知'");
patch("apiFmtQuotaNum((apiQuotaOf(modelId) || {}).autoToday || 0)", "apiFmtQuotaNum((apiQuotaOf(modelId) || {}).autoToday)");
patch("function apiFmtTokens(n) {\n  n = Number(n) || 0", "function apiFmtTokens(n) {\n  if (n == null || n === '') return '—'\n  n = Number(n) || 0");
patch("var allDays = ((d.all && d.all.days) || []).slice().sort(function (a, b) { return a.date < b.date ? -1 : 1 })", "codexWhaleHost.recordsPager(body, d, fillUsageRecordsWindow)\n  var allDays = ((d.all && d.all.days) || []).slice().sort(function (a, b) { return a.date < b.date ? -1 : 1 })");
patch("bubbleRenderModules(usageAlertModsResolved(usageTurnCostLines(), null, null, usageCostValue(amount)))", "bubbleRenderModules(codexWhaleHost.completionModules(usageAlertModsResolved(usageTurnCostLines(), null, null, codexWhaleHost.completionCost(amount))))");
const costTriggers = /if \(d\.turn !== null && d\.amount !== null\) \{\n\s+showCostBubble\(Number\(d\.amount\)\)\n\s+\}/g;
if ([...source.matchAll(costTriggers)].length !== 2) throw new Error("本轮消耗触发锚点已变化");
source = source.replace(costTriggers, "if (d.turn !== null) { codexWhaleHost.complete(d, showCostBubble) }");
patch("var nb = Number(data.totalBalance)", "var nb = data.totalBalance == null ? null : Number(data.totalBalance)");
patch("state.bonusBalance = isFinite(Number(data.bonusBalance)) ? Number(data.bonusBalance) : null", "state.bonusBalance = data.bonusBalance != null && isFinite(Number(data.bonusBalance)) ? Number(data.bonusBalance) : null");
patch("state.rechargeBalance = isFinite(Number(data.rechargeBalance)) ? Number(data.rechargeBalance) : null", "state.rechargeBalance = data.rechargeBalance != null && isFinite(Number(data.rechargeBalance)) ? Number(data.rechargeBalance) : null");
patch("state.isPeak = !!data.isPeak", "state.peakSupported = data.peakSupported === true\n        state.isPeak = data.isPeak === null ? null : !!data.isPeak");
patch("function bubblePeakText(m) {", "function bubblePeakText(m) {\n  if (state.peakSupported !== true) return '未配置峰谷'");
patch("function bubbleCountdownText() {", "function bubbleCountdownText() {\n  if (state.peakSupported !== true) return '未配置峰谷'");
patch("express()\nrender()\napplySoundSet()", "codexWhaleHost.bind({ root: root, menu: menuBox, refresh: refresh, showBubble: showBubble, playTaskEndSound: playTaskEndSound, pollWaitState: pollWaitState, pollLastTurn: pollLastTurn, openSoundSettings: openSoundSettingsPanel, openBubbleEditor: openBubbleEditor, openResources: openResManager })\nexpress()\nrender()\napplySoundSet()");
// 只改 Codex 适配说明；DeepSeek 作为可选厂商模板仍保留原名称。
source = source.replaceAll("DeepSeek（内置）", "当前供应商").replaceAll("DeepSeek 账户余额", "当前供应商账户余额").replaceAll("配置 DeepSeek API key", "配置当前供应商 API").replaceAll("DSH 数据目录", "Codex++ 数据目录").replaceAll("按本机 DSH 会话", "按本机 Codex 会话").replaceAll("DeepSeek 余额", "Codex 用量");
const generated = "// 此文件由 scripts/assemble-whale-runtime.mjs 从上游完整源码生成，请修改生成器。\n" + source;
const output = new URL("full-widget-codex.js", root);
if (process.argv.includes("--check")) {
  if (readFileSync(output, "utf8") !== generated) throw new Error("鲸鱼适配产物未更新，请运行 node scripts/assemble-whale-runtime.mjs");
} else writeFileSync(output, generated);
console.log(`鲸鱼完整运行时 ${process.argv.includes("--check") ? "校验通过" : "已生成"} (${Buffer.byteLength(generated)} bytes)`);
