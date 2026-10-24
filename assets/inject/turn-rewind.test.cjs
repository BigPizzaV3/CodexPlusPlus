const assert = require('node:assert/strict');
const fs = require('node:fs');
const vm = require('node:vm');
const path = require('node:path');
const source = fs.readFileSync(path.join(__dirname,'renderer-inject/85-turn-rewind-controller.js'),'utf8');

function makeApi() {
  const context = { module: {exports:{}}, setTimeout, clearTimeout, crypto: require('node:crypto').webcrypto };
  context.window = context;
  context.codexPlusBackendSettingsLoaded = true;
  context.codexPlusSettings = () => ({turnRewind:true});
  vm.runInNewContext(source, context);
  return context.__codexPlusTurnRewind;
}
function turn(id, prompt='', status='completed', items=[]) {
  return {turnId:id,status,params:{input:[{type:'text',text:prompt}]},items};
}
async function scenario({partial=false, malformed=false, concurrent=false, failed=false, readIncomplete=false, failBackup=false, legacy=false, revertFails=false, syncFails=false, timeout=false, disabled=false, follower=false, disabledBeforeRevert=false, disabledBeforePrompt=false, busy=false}={}) {
  const api=makeApi(), calls=[], saved=[];
  api.setEnabledCheck(() => !disabled);
  let turns=[turn('a','keep'),turn('b','remove'),turn('c','remove too')], listener;
  if(readIncomplete) turns[1].itemsPagination={hasLoadedOldest:false};
  const state={historyMode:legacy?'legacy':'paginated',resumeState:'resumed',threadRuntimeStatus:{type:busy?'active':'idle'},cwd:'/synthetic-workspace'};
  const manager={
    getHostId:()=> 'local',getConversation:()=>state,getStreamRole:()=>({role:follower?'follower':'owner'}),
    getCompleteConversationTurns:async()=>turns,
    loadRemainingTurnItems:async()=>{calls.push('hydrate');},
    addTurnCompletedListener:fn=>{listener=fn;return()=>{};},
    addConversationStateCallback:()=>()=>{},
    sendFollowUpMessage:async(id,options)=>{
      calls.push('prompt');
      assert.match(options.prompt,/目标轮次 ID：b/);
      const report={status:partial?'partial':'complete',undone:['file restored'],unresolved:partial?['missing backup']:[],verification:['checked']};
      const text=malformed?'done':`CODEX_REWIND_RESULT_test=${JSON.stringify(report)}`;
      turns.push(turn('undo',options.prompt,timeout?'inProgress':failed?'failed':'completed',[{type:'assistant',text,completed:true}]));
      if(concurrent)turns.push(turn('independent','new user message'));
      if(!timeout)await listener({conversationId:id,turnId:'undo'});
    },
    sendRequest:async(method,args)=>{
      assert.equal(method,'thread/revert');assert.equal(args.beforeTurnId,'b');
      calls.push('revert');if(revertFails)throw Error('server refused');
      return {thread:{id:'chat',sessionId:'chat'},turnsBackwardsCursor:null,itemsBackwardsCursor:null};
    },
    applyRevertResponseToConversation:options=>{
      if(syncFails)throw Error('renderer sync error');
      assert.deepEqual(Array.from(options.revertedTurns,t=>t.turnId),['b','c','undo']);
      calls.push('apply'); turns=turns.slice(0,1);
    },
    interruptConversation:async(id,mode,target)=>{assert.equal(target,'undo');calls.push('interrupt');}
  };
  const record=await api.run('chat','b',{manager,token:'test',timeoutMs:100,
    saveRun:async x=>{if(failBackup)throw Error('storage full');if((disabledBeforeRevert && x.status==='reverting') || (disabledBeforePrompt && x.status==='running'))api.setEnabledCheck(()=>false);saved.push(JSON.parse(JSON.stringify(x)));},
    panel:{update:()=>{}}});
  return {record,calls,turns,saved};
}
(async()=>{
  let s=await scenario();assert.equal(s.record.status,'complete');assert.deepEqual(s.calls,['prompt','revert','apply']);assert.deepEqual(s.turns.map(t=>t.turnId),['a']);assert.equal(s.saved[0].historyBefore.length,3);
  for(const options of [{partial:true},{malformed:true},{concurrent:true},{failed:true},{readIncomplete:true},{failBackup:true},{legacy:true},{disabled:true},{follower:true},{disabledBeforeRevert:true},{disabledBeforePrompt:true},{busy:true}]) {
    s=await scenario(options);assert.equal(s.record.status,'incomplete',JSON.stringify(options));assert.ok(!s.calls.includes('revert'),JSON.stringify(options));
  }
  s=await scenario({revertFails:true});assert.equal(s.record.status,'incomplete');assert.ok(!s.calls.includes('apply'));assert.equal(s.turns.length,4);
  s=await scenario({syncFails:true});assert.equal(s.record.status,'ui-refresh-required');assert.equal(s.record.historyTruncated,true);
  s=await scenario({timeout:true});assert.equal(s.record.status,'incomplete');assert.ok(s.calls.includes('interrupt'));assert.ok(!s.calls.includes('revert'));
  const api=makeApi();assert.throws(()=>api.parseResult('CODEX_REWIND_RESULT_x={"status":"complete"}','x'));
  assert.throws(()=>api.parseResult('CODEX_REWIND_RESULT_x={}\nCODEX_REWIND_RESULT_x={}','x'));
  console.log('PASS: successful truncation, full backup, partial rollback, malformed report, concurrent message, failed turn, incomplete history, backup failure, legacy history, server failure, UI synchronization failure, timeout interruption.');
})().catch(error=>{console.error(error);process.exitCode=1;});
