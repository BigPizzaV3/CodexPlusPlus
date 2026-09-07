import {test} from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import vm from 'node:vm';

const code=fs.readFileSync(new URL('src/native-sidechat.js',import.meta.url),'utf8').replace("import('app://-/assets/app-initial-f87238153a19.js')",'Promise.resolve(globalThis.native)');
const scope=id=>({get(){},value:{routeKind:'local-thread',conversationId:id}});
const scopeFiber=s=>({memoizedState:{memoizedState:{current:s}}});
function runtime(elements){
  const parent={cwd:'D:/project'},manager={getConversation:id=>id==='target'?parent:null};
  const diagnostics=[];
  const context=vm.createContext({native:{kr(){},Mr(){throw Error('No submission allowed');},ESt:()=>manager},document:{querySelectorAll:()=>elements},diagnostic:(_,value)=>diagnostics.push(value)});
  vm.runInContext(code,context);
  return {context,manager,diagnostics};
}

test('find React context on a wrapper when editor DOM is owned by ProseMirror',async()=>{
  const route=scopeFiber(scope('target'));
  const callback=()=>{};
  const composer={return:route,memoizedProps:{onCreateSideConversation:callback}};
  const wrapper={'__reactFiber$test':{return:composer}};
  const editor={parentElement:{parentElement:wrapper}}; // No React expando on editor.
  const {context,manager,diagnostics}=runtime([editor]);
  const result=await context.nativeContext('target');
  assert.equal(result.manager,manager);assert.equal(result.create,callback);
  assert.equal(diagnostics[0].managerFound,true);
});

test('read history from route sibling when input is absent, without requiring side chat callback',async()=>{
  const route=scopeFiber(scope('target'));
  const root={child:route};route.return=root;
  const header={return:root};route.sibling=header;
  const {context}=runtime([{'__reactFiber$test':header}]);
  const result=await context.nativeContext('target');
  assert.equal(result.scope.value.conversationId,'target');assert.equal(result.create,null);
});

test('choose matching route callback rather than another task composer',async()=>{
  const target=scopeFiber(scope('target')),other=scopeFiber(scope('other'));
  const expected=()=>{},wrong=()=>{};
  const targetComposer={return:target,memoizedProps:{onCreateSideConversation:expected}};
  const otherComposer={return:other,memoizedProps:{onCreateSideConversation:wrong}};
  const {context}=runtime([{'__reactFiber$a':otherComposer},{'__reactFiber$b':targetComposer}]);
  assert.equal((await context.nativeContext('target')).create,expected);
});

test('root container uses committed current tree and ignores a hidden route',async()=>{
  const route=scopeFiber(scope('target'));
  const hidden={tag:22,memoizedState:{},child:scopeFiber(scope('other')),sibling:route};
  const committed={child:hidden};hidden.return=committed;route.return=committed;
  const old={stateNode:{current:committed},child:scopeFiber(scope('stale'))};
  const {context,diagnostics}=runtime([{'__reactContainer$test':old}]);
  assert.equal((await context.nativeContext('target')).scope.value.conversationId,'target');
  assert.equal(diagnostics[0].scopeCount,1);
});

test('absence of a matching context fails without requesting input-box focus or submitting anything',async()=>{
  const {context}=runtime([{'__reactFiber$test':scopeFiber(scope('other'))}]);
  await assert.rejects(context.nativeContext('target'),/原生接口尚未就绪/);
});

test('virtualized navigation belongs to the requested task and selected route scope',async()=>{
  const s=scope('target'),route=scopeFiber(s),calls=[];
  const adapter={scrollToTurn:async id=>calls.push(id),getTurnContainer:()=>null};
  const wrong={scrollToTurn:()=>{throw Error('wrong task');},getTurnContainer:()=>null};
  route.child={return:route,memoizedState:{memoizedState:[wrong,['other',s]],next:{memoizedState:[adapter,['target',s]]}}};
  const {context}=runtime([{'__reactFiber$test':route}]);
  const result=await context.nativeContext('target');await result.navigation.scrollToTurn('folded-turn');assert.deepEqual(calls,['folded-turn']);
  route.child.memoizedState.next.memoizedState[1][1]=scope('target');
  assert.equal((await context.nativeContext('target')).navigation,adapter);
  route.child.memoizedState.next.memoizedState[1][1]={get(){},value:{}};
  assert.equal((await context.nativeContext('target')).navigation,adapter);
  route.child.memoizedState.next.memoizedState[1][0]='other';
  assert.equal((await context.nativeContext('target')).navigation,undefined);
});

test('native message targets match complete encoded item IDs, including grouped items',()=>{
  const {context}=runtime([]),id='item/a b',right={getAttribute:()=>`first ${encodeURIComponent(id)} last`},wrong={getAttribute:()=>`${encodeURIComponent(id)}-different`};
  const doc={getElementById:()=>null,querySelectorAll:()=>[wrong,right]};
  assert.equal(context.nativeMessageTarget(doc,id),right);
  assert.equal(context.nativeMessageTarget(doc,'missing'),null);
});
