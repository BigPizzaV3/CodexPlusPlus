import test from 'node:test';
import assert from 'node:assert/strict';
import {nativeAssetCandidates,bindNativeRuntime,createNativeLoader} from './native-runtime.mjs';
const current='app://-/assets/app-initial-bcc2ff475eb6.js';
test('discovers the loaded application asset without accepting external scripts',()=>{
  const doc={querySelectorAll:()=>[{src:current},{src:'https://example.org/app-initial-bad.js'}]};
  assert.deepEqual(nativeAssetCandidates(doc,{getEntriesByType:()=>[{name:current}]}),[current]);
});
test('current build supports reading and live HTTP initialization independently of submission',()=>{
  const module={EDt(){},MDt(){},hJt(){module.mJt={getInstance:()=>({fetch(){}})};module.TW={httpFetch:{}};}};
  const native=bindNativeRuntime(module,current.split('/').pop());
  assert.equal(native.ESt,module.EDt);assert.equal(native.kr,module.MDt);
  assert.equal(native.Mr,undefined);native.lGt();assert.equal(typeof native.cGt.getInstance().fetch,'function');assert.ok(native.jR.httpFetch);
});
test('loads current discovered asset once for concurrent callers',async()=>{
  const calls=[];const loader=createNativeLoader({discover:()=>[current],importModule:async url=>{calls.push(url);return {};}});
  const [a,b]=await Promise.all([loader(),loader()]);assert.equal(a,b);assert.deepEqual(calls,[current]);
});
test('failed imports can be retried and hide the obsolete hashed-path error',async()=>{
  let fails=true;const loader=createNativeLoader({discover:()=>[current],importModule:async()=>{if(fails)throw Error('Failed to fetch dynamically imported module');return {};}});
  await assert.rejects(loader(),/原生模块加载失败/);fails=false;assert.ok(await loader());
});
test('unknown builds never receive guessed export aliases or stale build fallback',async()=>{
  let calls=0;const loader=createNativeLoader({discover:()=>['app://-/assets/app-initial-new.js'],importModule:async()=>{calls++;}});
  await assert.rejects(loader(),/尚未适配/);assert.equal(calls,0);
});
test('missing resource timing falls back across reviewed builds',async()=>{
  const calls=[];const loader=createNativeLoader({discover:()=>[],importModule:async url=>{calls.push(url);if(url===current)throw Error('missing');return {ESt:'old'};}});
  assert.equal((await loader()).ESt,'old');assert.equal(calls.length,2);
});
