import test from 'node:test';
import assert from 'node:assert/strict';
import {parseMessage,buildGraph} from './model.mjs';
const record=(role,text,extra={})=>({type:'response_item',timestamp:'2026-09-06T00:00:00Z',payload:{type:'message',role,id:'m1',content:[{text}],...extra}});
test('排除系统、工具与环境内容，仅采集用户及助手消息',()=>{
  assert.equal(parseMessage(record('developer','private')),null);
  assert.equal(parseMessage(record('user','<recommended_plugins>environment')),null);
  assert.equal(parseMessage({type:'function_call',payload:{}}),null);
  assert.equal(parseMessage(record('user','换一个方案')).text,'换一个方案');
});
test('澄清回答解包，保留稳定来源 ID',()=>{
  const m=parseMessage(record('user','<send_user_message_question_reply>[{"answer":"仓库地址"}]</send_user_message_question_reply>'));
  assert.equal(m.id,'m1');assert.equal(m.text,'仓库地址');
});
test('无来源节点被剔除，新消息保持待整理，不臆造失败分支',()=>{
  const messages=[parseMessage(record('user','这个方法不行，换方案 B'))];
  const result=buildGraph(messages,{nodes:[{id:'bad',sources:['absent']}]});
  assert.equal(result.nodes.length,1);assert.equal(result.nodes[0].status,'待整理');assert.equal(result.nodes[0].lane,'main');
});
test('保留已整理分支与来源，进展不制造新决策节点',()=>{
  const m=parseMessage(record('assistant','继续检查',{phase:'commentary'}));
  const annotations={nodes:[{id:'root',lane:'main',sources:['m1']},{id:'b',parent:'root',lane:'branch',sources:['m1']}]};
  assert.equal(buildGraph([m]).nodes.length,0);
  const graph=buildGraph([m],annotations);assert.deepEqual(graph.edges,[{from:'root',to:'b',kind:'branch'}]);
});
