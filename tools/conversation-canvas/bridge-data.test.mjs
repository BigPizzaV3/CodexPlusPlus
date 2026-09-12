import test from 'node:test';
import assert from 'node:assert/strict';
import {graphFromExport} from './bridge-data.mjs';
const id='11111111-1111-1111-1111-111111111111';
const message=(text,messageId)=>({type:'response_item',timestamp:'2026-09-06',payload:{type:'message',role:'user',id:messageId,content:[{text}]}});
const result=records=>({status:'ok',kind:'codex-rollout',session_id:id,content:records.map(r=>JSON.stringify(r)+'\n').join('')});
test('原生桥接解析保留来源、分支，过滤工具输出和未完成尾行',()=>{
  const input=result([message('先尝试方案 A','m1'),{type:'response_item',payload:{type:'function_call',name:'private'}},message('先尝试方案 A','m1')]);
  input.content+='{"type":"response_item"';
  const graph=graphFromExport(input,id,{nodes:[{id:'a',lane:'main',sources:['m1']},{id:'b',lane:'branch',parent:'a',sources:['m1']}]});
  assert.equal(graph.messages.length,1);assert.equal(graph.messages[0].id,'m1');assert.equal(graph.nodes.length,2);assert.equal(graph.edges[0].kind,'branch');
});
test('接口失败与任务不匹配必须明确报错',()=>{
  assert.throws(()=>graphFromExport({status:'failed',message:'offline'},id),/offline/);
  assert.throws(()=>graphFromExport({...result([]),session_id:'wrong'},id),/不匹配/);
  assert.throws(()=>graphFromExport(result([{type:'session_meta',payload:{id:'wrong'}}]),id),/不一致/);
});
test('没有消息 ID 时，追加会话记录不会改变已有来源 ID',()=>{
  const input=result([message('旧消息')]);const before=graphFromExport(input,id);
  input.content+=JSON.stringify(message('新消息'))+'\n';const after=graphFromExport(input,id);
  assert.equal(before.messages[0].id,after.messages[0].id);assert.notEqual(after.messages[0].id,after.messages[1].id);
});
