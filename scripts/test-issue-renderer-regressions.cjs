const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');
const root = path.resolve(__dirname, '..');
const output = process.env.CODEX_ISSUE_TEST_OUTPUT;
if (output) fs.mkdirSync(output, { recursive: true });
const shard = name => fs.readFileSync(path.join(root, 'assets/inject/renderer-inject', name), 'utf8');
function extract(source, name) {
  const result = source.match(new RegExp(`^  (?:async )?function ${name}\\([^]*?^  \\}`, 'm'));
  assert.ok(result, name);
  return result[0];
}

async function diagnostics() {
  const source = shard('60-plugin-marketplace.js');
  for (const mode of ['success', 'rejected', 'failed-status', 'throw', 'absent', 'beacon-failed']) {
    const calls = { bridge: 0, beacon: 0, fetch: 0 };
    const scope = {
      helperBase: 'http://127.0.0.1:12345', Blob, JSON,
      codexPlusDiagnosticPayload: (event, detail) => ({ event, detail }),
      navigator: { sendBeacon: () => { calls.beacon++; return mode !== 'beacon-failed'; } },
      fetch: async () => { calls.fetch++; return {}; }, window: {},
    };
    if (mode !== 'absent' && mode !== 'beacon-failed') scope.window.__codexSessionDeleteBridge = () => {
      calls.bridge++;
      if (mode === 'throw') throw new Error('fixture');
      if (mode === 'rejected') return Promise.reject(new Error('fixture'));
      return Promise.resolve({ status: mode === 'success' ? 'ok' : 'failed' });
    };
    vm.createContext(scope);
    vm.runInContext(extract(source, 'sendCodexPlusDiagnostic'), scope);
    await scope.sendCodexPlusDiagnostic('fixture', {});
    assert.equal(calls.beacon, mode === 'success' ? 0 : 1, mode);
    assert.equal(calls.fetch, mode === 'beacon-failed' ? 1 : 0, mode);
    assert.equal(calls.bridge, ['absent', 'beacon-failed'].includes(mode) ? 0 : 1, mode);
  }
  console.log('diagnostic bridge/fallback: 6 cases passed');
}

async function browserChecks() {
  const { chromium } = require('playwright');
  const browser = await chromium.launch({ headless: true, executablePath: process.env.CODEX_TEST_BROWSER_PATH });
  try {
    const page = await browser.newPage();
    const compatibility = fs.readFileSync(path.join(root, 'assets/inject/dream-skin-native-shell-compat.css'), 'utf8');
    const artwork = 'data:image/png;base64,' + fs.readFileSync(path.join(root, 'assets/inject/dream-skin-default.png')).toString('base64');
    let count = 0;
    for (const os of ['darwin', 'win32']) for (const mode of ['ambient', 'full']) for (const image of ['builtin', 'managed', 'selected']) {
      const base = fs.readFileSync(path.join(root, `assets/inject/upstream/dream-skin/${os === 'darwin' ? 'macos' : 'windows'}/dream-skin.css`), 'utf8');
      await page.setContent(`<style>@layer theme,base,components,utilities;@layer utilities{.bg-token-main-surface-primary{background:rgb(240,240,240)!important}}#surface{width:500px;height:120px}#art{position:absolute;inset:0;width:500px;height:120px;z-index:-1}</style><style>${base}</style>
        <main data-app-shell-main-surface><img id="art" src="${artwork}"><div id="surface" class="bg-token-main-surface-primary h-full w-full"></div>
        <div data-codex-composer-root><div id="composer" class="bg-token-main-surface-primary h-full w-full"></div></div>
        <div role="dialog"><div id="dialog" class="bg-token-main-surface-primary h-full w-full"></div></div></main>`);
      await page.evaluate(({os,mode,image}) => {
        Object.assign(document.documentElement.dataset, {dreamSkin:'active',codexOs:os,dreamTaskMode:mode,dreamArtWide:'true',fixtureImage:image});
      }, { os, mode, image });
      assert.ok(await page.$eval('#art', async node => { await node.decode(); return node.naturalWidth > 0; }), 'fixture PNG decodes');
      const color = id => page.$eval(id, node => getComputedStyle(node).backgroundColor);
      assert.equal(await color('#surface'), 'rgb(240, 240, 240)', 'fixture reproduces layered native obstruction');
      if (output && os === 'darwin' && mode === 'ambient' && image === 'builtin') await page.screenshot({ path: path.join(output, 'native-layer-before.png') });
      await page.addStyleTag({content: compatibility});
      assert.equal(await color('#surface'), 'rgba(0, 0, 0, 0)', `${os}/${mode}/${image}`);
      for (const id of ['#composer', '#dialog']) assert.equal(await color(id), 'rgb(240, 240, 240)', id);
      if (output && os === 'darwin' && mode === 'ambient' && image === 'builtin') await page.screenshot({ path: path.join(output, 'native-layer-after.png') });
      await page.evaluate(() => document.documentElement.removeAttribute('data-dream-skin'));
      assert.equal(await color('#surface'), 'rgb(240, 240, 240)', 'disabled skin restores native surface');
      count++;
    }
    console.log(`real computed-style layer matrix with PNG decode: ${count} cases passed`);
    if (output) fs.writeFileSync(path.join(output, 'results.json'), JSON.stringify({ browser: browser.version(),
      themeCascadeCases: count, fixturePngDecoded: true, diagnosticsCases: 6,
      scope: 'Synthetic DOM with repository base CSS and important native utility fixture; not real Windows MSIX acceptance.' }, null, 2));
  } finally { await browser.close(); }
}

(async () => {
  if (process.argv.includes('--browser')) await browserChecks(); else await diagnostics();
})().catch(error => { console.error(error); process.exit(1); });
