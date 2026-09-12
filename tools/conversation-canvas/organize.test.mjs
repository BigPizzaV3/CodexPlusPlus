import {test} from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import vm from 'node:vm';
import {organizationPrompt,parseOrganization,validateOrganization} from './organize.mjs';
const messages=[{id:'m1',role:'user',text:'尝试一'},{id:'m2',role:'assistant',text:'尚未验证'}];
const value=()=>({requestId:'request',nodes:[{id:'n1',parent:null,lane:'main',title:'目标',summary:'摘要',description:'具体描述',status:'待验证',sources:['m1','m2']}]});
const adapterCode=()=>fs.readFileSync(new URL('src/native-sidechat.js',import.meta.url),'utf8').replace("import('app://-/assets/app-initial-f87238153a19.js')",'Promise.resolve(globalThis.native)');
const uiNative={rMt(){throw Error('Must not open sidebar');},x3(){throw Error('Must not attach visible side task');},w3(){throw Error('Must not mutate sidebar');}};
const assertModel=mode=>{assert.equal(mode.mode,'default');assert.equal(mode.settings.model,'gpt-5.6-luna');assert.equal(mode.settings.reasoning_effort,'medium');assert.equal(mode.settings.developer_instructions,null);};
test('parse grounded output and reject invented sources, duplicate IDs, wrong request and cycles',()=>{
  assert.equal(parseOrganization('```json\n'+JSON.stringify(value())+'\n```',messages,'request').nodes.length,1);
  for(const change of [v=>v.nodes[0].sources=['invented'],v=>v.nodes.push(v.nodes[0]),v=>v.requestId='other',v=>v.nodes[0].parent='n1',v=>v.nodes[0].lane='branch']){
    const v=value();change(v);assert.throws(()=>validateOrganization(v,messages,'request'));
  }
});
test('prompt includes original IDs and enforces read-only side task; oversized inputs are explicit',()=>{
  assert.match(organizationPrompt(messages,'request'),/不要调用工具/);
  assert.match(organizationPrompt(messages,'request'),/m2/);
  assert.throws(()=>organizationPrompt([{text:'x'.repeat(240001)}],'request'),/过长/);
});
test('task tree permits nested branches and forward references while rejecting orphan and disconnected cycles',()=>{
  const make=(id,parent,lane='branch')=>({...value().nodes[0],id,parent,lane});
  const valid={requestId:'request',currentNodeId:'improve',nodes:[make('root',null,'main'),make('improve','experiment'),make('experiment','a'),make('a','root'),make('b','root')]};
  const tree=validateOrganization(valid,messages,'request');
  assert.equal(tree.schemaVersion,2);assert.equal(tree.currentNodeId,'improve');
  assert.equal(tree.nodes.find(n=>n.id==='experiment').parent,'a');
  for(const nodes of [[make('root',null,'main'),make('a','missing')],[make('root',null,'main'),make('a','b'),make('b','a')],[make('root',null,'main'),make('root2',null,'main')]]){
    assert.throws(()=>validateOrganization({requestId:'request',nodes},messages,'request'));
  }
  assert.throws(()=>validateOrganization({...valid,currentNodeId:'missing'},messages,'request'),/当前推进/);
});
test('native adapter submits only to verified ephemeral side conversation and reads its completed turn',async()=>{
  const parent={cwd:'D:/project',latestCollaborationMode:{mode:'default',settings:{model:'current'}}};
  const side={sideConversation:true,ephemeral:true,turns:[]};
  const manager={getHostId:()=> 'local',getConversation:id=>id==='parent'?parent:side};
  const scope={get(){},value:{routeKind:'local-thread',conversationId:'parent'}};
  let submitted=0;
  const create=async()=>{throw Error('failed to prepare paginated fork: expected ordinal 638, got 637');};
  const fiber={memoizedProps:{onCreateSideConversation:create},memoizedState:{memoizedState:{current:scope}}};
  const native={...uiNative,kr(){},ESt:()=>manager,Q1:async(s,host,options,hooks)=>{
    assert.equal(s,scope);assert.equal(host,'local');assert.deepEqual(Array.from(options.input),[]);
    assert.ok(options.sideConversation);assert.equal(options.sourceConversationId,undefined);assertModel(options.collaborationMode);
    await hooks.afterConversationCreated('side');return {status:'created',conversationId:'side',firstTurn:{status:'not-requested'}};
  },Mr:async options=>{
    assert.equal(options.targetConversationId,'side');assert.equal(options.context.prompt,'organize');
    assert.equal(options.shouldSendPermissionOverrides,false);assertModel(options.activeCollaborationMode);assertModel(options.context.collaborationMode);assert.equal(parent.latestCollaborationMode.settings.model,'current');
    submitted++;side.turns=[{turnId:'turn',status:'completed',items:[{type:'agentMessage',phase:'final_answer',text:'result'}]}];return 'turn';
  }};
  const context=vm.createContext({native,document:{querySelectorAll:()=>[{'__reactFiber$test':fiber}]},setTimeout,Date,crypto});
  const code=adapterCode();
  vm.runInContext(code,context);
  assert.equal(await context.nativeSideChat('parent','organize',()=>{}),'result');
  assert.equal(submitted,1);
  side.ephemeral=false;
  await assert.rejects(context.nativeSideChat('parent','organize',()=>{}),/隔离检查/);
  assert.equal(submitted,1);
});

test('native side-chat resumes accepted work without duplicate submission, and rotates after eight batches',async()=>{
  const parent={cwd:'D:/project'},sides=new Map(),session={};let submissions=0,created=0,checkpoint;
  const scope={get(){},value:{routeKind:'local-thread',conversationId:'parent'}};
  const manager={getHostId:()=> 'local',getConversation:id=>id==='parent'?parent:sides.get(id)};
  const controller=new AbortController();
  const create=async()=>{throw Error('failed to prepare paginated fork: expected ordinal 638, got 637');};
  const fiber={memoizedProps:{onCreateSideConversation:create},memoizedState:{memoizedState:{current:scope}}};
  const native={...uiNative,kr(){},ESt:()=>manager,Q1:async(s,host,options,hooks)=>{
    const id='side'+(++created);sides.set(id,{ephemeral:true,sideConversation:true,turns:[]});await hooks.afterConversationCreated(id);return {status:'created',conversationId:id,firstTurn:{status:'not-requested'}};
  },Mr:async options=>{
    assert.notEqual(options.targetConversationId,'parent');
    const turn={turnId:'turn'+(++submissions),clientUserMessageId:options.clientUserMessageId,status:'completed',items:[{type:'agentMessage',phase:'final_answer',text:'batch result'}]};
    sides.get(options.targetConversationId).turns.push(turn);
    if(submissions===1)controller.abort();return turn.turnId;
  }};
  const code=adapterCode();
  const context=vm.createContext({native,document:{querySelectorAll:()=>[{'__reactFiber$test':fiber}]},setTimeout,Date,crypto});vm.runInContext(code,context);
  const opts={session,requestId:'r1',onSession:async()=>{checkpoint=structuredClone(session);}};
  await assert.rejects(context.nativeSideChat('parent','organize',()=>{},controller.signal,opts),{name:'AbortError'});
  assert.equal(checkpoint.turnId,'turn1');assert.equal(submissions,1);
  assert.equal(await context.nativeSideChat('parent','organize',()=>{},undefined,opts),'batch result');assert.equal(submissions,1);
  // A checkpoint may be lost immediately after the native submit accepted it.
  Object.assign(session,{phase:'submitting',turnId:null});
  await context.nativeSideChat('parent','organize',()=>{},undefined,opts);assert.equal(submissions,1);
  session.batches=1;
  for(let i=2;i<=9;i++)await context.nativeSideChat('parent','organize',()=>{},undefined,{...opts,requestId:'r'+i});
  assert.equal(submissions,9);assert.equal(created,2);
  Object.assign(session,{requestId:'unconfirmed',messageId:'missing',phase:'submitting',turnId:null});
  await assert.rejects(context.nativeSideChat('parent','organize',()=>{},undefined,{...opts,requestId:'unconfirmed'}),/尚未确认/);assert.equal(submissions,9);
});

test('background creation needs no composer or UI exports and overrides legacy model sessions',async()=>{
  const parent={cwd:'D:/project'},sides=new Map([['legacy',{ephemeral:true,sideConversation:true,turns:[]}]]),session={threadId:'parent',sideId:'legacy',turnId:'old',requestId:'batch',phase:'waiting'};let starts=0,submits=0;
  const scope={get(){},value:{routeKind:'local-thread',conversationId:'parent',pathname:'/thread/parent'}};
  const manager={getHostId:()=> 'local',getConversation:id=>id==='parent'?parent:sides.get(id)};
  const native={kr(){},ESt:()=>manager,
    Q1:async(s,host,options,hooks)=>{starts++;assertModel(options.collaborationMode);sides.set('blank',{ephemeral:true,sideConversation:true,turns:[]});await hooks.afterConversationCreated('blank');return {status:'created',conversationId:'blank',firstTurn:{status:'not-requested'}};},
    Mr:async options=>{submits++;assert.equal(options.targetConversationId,'blank');assertModel(options.context.collaborationMode);sides.get('blank').turns.push({turnId:'t',status:'completed',items:[{type:'agentMessage',text:'result'}]});return 't';}};
  const context=vm.createContext({native,document:{querySelectorAll:()=>[{'__reactFiber$test':{memoizedState:{memoizedState:{current:scope}}}}]},setTimeout,crypto});vm.runInContext(adapterCode(),context);
  const options={session,requestId:'batch',onSession:async()=>{}};
  assert.equal(await context.nativeSideChat('parent','prompt',()=>{},undefined,options),'result');assert.equal(starts,1);assert.equal(submits,1);
  assert.equal(session.profile,'gpt-5.6-luna:medium:silent');
  assert.equal(await context.nativeSideChat('parent','prompt',()=>{},undefined,options),'result');assert.equal(submits,1);
  native.Mr=async options=>{assertModel(options.context.collaborationMode);throw Error('model unavailable');};
  await assert.rejects(context.nativeSideChat('parent','prompt',()=>{},undefined,{...options,requestId:'next'}),/model unavailable/);
});

test('uncertain blank-side creation blocks duplicate creation until native settlement supplies identity',async()=>{
  const parent={cwd:'D:/project'},sides=new Map(),session={};let starts=0,hooks;
  const scope={get(){},value:{routeKind:'local-thread',conversationId:'parent'}};
  const manager={getHostId:()=> 'local',getConversation:id=>id==='parent'?parent:sides.get(id)};
  const native={...uiNative,kr(){},ESt:()=>manager,Q1:async(s,h,p,callbacks)=>{starts++;hooks=callbacks;return {status:'outcome-unknown'};},Mr:async()=>{throw Error('must not submit');}};
  const context=vm.createContext({native,document:{querySelectorAll:()=>[{'__reactFiber$test':{memoizedState:{memoizedState:{current:scope}}}}]},setTimeout,crypto});vm.runInContext(adapterCode(),context);
  const options={session,onSession:async()=>{}};
  await assert.rejects(context.nativeSideChat('parent','prompt',()=>{},undefined,options),/尚未创建完成/);
  await assert.rejects(context.nativeSideChat('parent','prompt',()=>{},undefined,options),/尚未确认/);assert.equal(starts,1);
  sides.set('accepted',{ephemeral:true,sideConversation:true,turns:[]});await hooks.onSettled({status:'created',conversationId:'accepted'});
  assert.equal(session.sideId,'accepted');assert.equal(session.phase,'ready');
});
