import {test} from 'node:test';
import assert from 'node:assert/strict';
import {readNativeHistory,isExportSizeError} from './native-history.mjs';
import {buildGraph} from './model.mjs';

test('size limit detection does not hide unrelated errors',()=>{
  assert.equal(isExportSizeError('会话文件超过分享大小限制'),true);
  assert.equal(isExportSizeError('Permission denied'),false);
});

test('durable per-page history checkpoints resume after a failed read without losing or duplicating messages',async()=>{
  const disk=new Map(),cache={get:async key=>structuredClone(disk.get(key)),set:async(key,value)=>{disk.set(key,structuredClone(value));},delete:async key=>disk.delete(key)};
  let fail=true;const seen=[];
  const send=async(method,p)=>{
    if(method==='thread/turns/list')return {data:[{id:'t',status:'completed'}]};
    seen.push(p.cursor);
    if(p.cursor==='p2'&&fail)throw Error('network interrupted');
    const id=p.cursor===null?'first':'last';
    return {data:[{type:'userMessage',id,content:[{text:id==='last'?'末尾消息'.repeat(80000):'最初方案'}]}],nextCursor:p.cursor===null?'p2':null};
  };
  await assert.rejects(readNativeHistory(send,'thread',cache),/interrupted/);
  assert.equal(disk.get('thread:t:partial').cursor,'p2');
  fail=false;const messages=await readNativeHistory(send,'thread',cache);
  assert.deepEqual(seen,[null,'p2','p2']);assert.deepEqual(messages.map(m=>m.id),['first','last']);assert.equal(messages[1].text.length,320000);
  assert.equal(disk.has('thread:t:partial'),false);
});
test('paged history includes early and latest messages, filters large tools and reuses completed turns',async()=>{
  let itemCalls=0;const cache=new Map();
  const send=async(method,p)=>{
    if(method==='thread/turns/list')return p.cursor===null?{data:[{id:'t1',status:'completed'}],nextCursor:'older'}:{data:[{id:'t2',status:'inProgress'}],nextCursor:null};
    itemCalls++;
    if(p.turnId==='t2')return {data:[{turnId:'t2',item:{type:'agentMessage',id:'a2',text:'最新回复'}}]};
    return p.cursor===null?{data:[{turnId:'t1',item:{type:'commandExecution',id:'tool',output:'x'.repeat(17*1024*1024)}},{turnId:'t1',item:{type:'userMessage',id:'u1',content:[{type:'text',text:'最初的目标'}]}}],nextCursor:'items-2'}:{data:[{turnId:'t1',item:{type:'agentMessage',id:'a1',text:'最初的结果',phase:'final_answer'}}]};
  };
  const first=await readNativeHistory(send,'thread',cache);
  assert.deepEqual(first.map(m=>m.id),['u1','a1','a2']);
  assert.equal(buildGraph(first,{nodes:[{id:'saved',lane:'main',sources:['u1']}]}).nodes[0].id,'saved');
  assert.equal(itemCalls,3);
  assert.deepEqual(await readNativeHistory(send,'thread',cache),first);
  assert.equal(itemCalls,4);
});
test('oversized item pages reduce page size and do not silently return empty history',async()=>{
  const sizes=[];
  const result=await readNativeHistory(async(method,p)=>{
    if(method==='thread/turns/list')return {data:[{id:'t',status:'completed'}]};
    sizes.push(p.limit);if(p.limit>8)throw Error('decoded message length too large');
    return {data:[{type:'userMessage',id:'u',content:[{text:'内容'}]}]};
  },'thread');
  assert.deepEqual(sizes,[32,16,8]);assert.equal(result.length,1);
});

test('legacy completed-turn caches regain navigation metadata from the current turn directory',async()=>{
  const cache=new Map([['thread:old-turn',[{id:'old-message',role:'assistant',text:'已完成的尝试',phase:'commentary'}]]]);
  const result=await readNativeHistory(async method=>{assert.equal(method,'thread/turns/list');return {data:[{id:'old-turn',status:'completed'}]};},'thread',cache);
  assert.equal(result[0].turnId,'old-turn');assert.equal(result[0].turnStatus,'completed');assert.equal(result[0].id,'old-message');
});
test('repeated cursors, foreign turns and cancelled reads fail explicitly',async()=>{
  await assert.rejects(readNativeHistory(async()=>({data:[],nextCursor:'same'}),'thread'),/游标重复/);
  await assert.rejects(readNativeHistory(async method=>method==='thread/turns/list'?{data:[{id:'t'}]}:{data:[{turnId:'wrong',item:{}}]},'thread'),/不匹配/);
  const controller=new AbortController();controller.abort();
  await assert.rejects(readNativeHistory(()=>{throw Error('must not call');},'thread',new Map(),()=>{},controller.signal),{name:'AbortError'});
});
