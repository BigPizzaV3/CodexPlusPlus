const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');
const source = fs.readFileSync(path.join(__dirname, 'renderer-inject/85-turn-rewind-controller.js'), 'utf8');

function harness() {
  const nodes = [];
  const document = {
    createElement(tag) {
      const node = { tag, style: {}, attributes: {}, children: [], disabled: false,
        setAttribute(name, value) { this.attributes[name] = value; }, append(...children) { this.children.push(...children); }, remove() {} };
      nodes.push(node);
      return node;
    },
    body: { appendChild() {} }
  };
  const context = { module: { exports: {} }, document, TextEncoder, setTimeout, clearTimeout,
    crypto: require('node:crypto').webcrypto };
  context.window = context;
  context.codexPlusBackendSettingsLoaded = true;
  context.codexPlusSettings = () => ({turnRewind:true});
  vm.runInNewContext(source, context);
  return { api: context.__codexPlusTurnRewind, nodes, context };
}
async function panel() {
  const h = harness();
  await h.api.run('synthetic-thread', 'synthetic-turn', {
    manager: { getHostId: () => 'local', getConversation: () => ({ historyMode: 'legacy', resumeState: 'resumed' }) },
    token: 'synthetic-export', saveRun: async () => {}
  });
  h.button = h.nodes.find(n => n.textContent === '导出恢复记录');
  h.status = h.nodes.find(n => n.tag === 'div' && n.attributes.role === 'status');
  return h;
}
(async () => {
  const record = { id: 'synthetic-id', note: '中文恢复记录', result: { status: 'partial' } };
  let h = harness(), captured;
  h.api.setSaveCopy(async request => { captured = request; return { path: '/synthetic/saved.json' }; });
  const result = await h.api.exportRecord(record);
  assert.equal(result.path, '/synthetic/saved.json');
  assert.equal(captured.fileName, 'codex-rewind-synthetic-id.json');
  assert.deepEqual(JSON.parse(Buffer.from(captured.bytes).toString('utf8')), record);
  h.api.setSaveCopy(async () => ({ path: null }));
  assert.equal((await h.api.exportRecord(record)).cancelled, true);
  h.api.setSaveCopy(async () => ({}));
  await assert.rejects(h.api.exportRecord(record), /未确认/);
  h.api.setSaveCopy(null);
  await assert.rejects(h.api.exportRecord(record), /文件保存接口/);

  h = await panel();
  let finish, calls = 0;
  h.api.setSaveCopy(() => { calls++; return new Promise(resolve => { finish = resolve; }); });
  const first = h.button.onclick();
  assert.equal(h.button.disabled, true);
  await h.button.onclick();
  assert.equal(calls, 1);
  finish({ path: '/synthetic/saved.json' });
  await first;
  assert.match(h.status.textContent, /已保存到/);
  assert.equal(h.button.disabled, false);
  const rollbackMessage = h.nodes.find(n => n.textContent?.includes('此会话格式不支持可靠截断'));
  assert.ok(rollbackMessage);
  const before = rollbackMessage.textContent;
  h.api.setSaveCopy(async () => { throw Error('synthetic disk failure'); });
  await h.button.onclick();
  assert.match(h.status.textContent, /导出失败.*synthetic disk failure/);
  assert.equal(rollbackMessage.textContent, before);
  assert.equal(h.button.disabled, false);
  h.api.setSaveCopy(async () => ({ path: null }));
  await h.button.onclick();
  assert.match(h.status.textContent, /已取消/);
  h.api.setSaveCopy(async () => ({ path: '/synthetic/retried.json' }));
  await h.button.onclick();
  assert.match(h.status.textContent, /retried.json/);

  console.log('PASS: UTF-8 export, native save result, cancellation, malformed result, missing bridge, visible errors, retry, duplicate-click protection, rollback-status preservation.');
})().catch(error => { console.error(error); process.exitCode = 1; });
