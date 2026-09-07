import {test} from 'node:test';
import assert from 'node:assert/strict';
import {organizeLong,prepareHistory,batchPrompt,mergeBatch} from './long-organizer.mjs';
import {buildGraph,visibleTreeRows} from './model.mjs';
import {treeMarkup} from './tree-markup.mjs';
import {transcriptPage} from './view-pages.mjs';

const msg=(id,text)=>({id,role:'user',text});
function decode(prompt){
  return {requestId:prompt.match(/"requestId":"([^"]+)"/)[1],prefix:prompt.match(/新 ID 必须以 (b\d+_) 开头/)[1],
    catalog:JSON.parse(prompt.split('已有节点目录（摘要可能缩短，以原 ID 为准）：')[1].split('\n')[0]),
    parts:JSON.parse(prompt.split('本批资料：')[1].split('\n')[0])};
}
function answer(prompt,{onePerPart=false}={}){
  const {requestId,prefix,catalog,parts}=decode(prompt),root=catalog.find(n=>n.parent===null);
  const make=(id,parent,sources)=>({id,parent,lane:parent?'branch':'main',title:'任务 '+id,summary:'依据 '+sources.length,description:'真实批次资料归纳',status:'待验证',sources});
  const upserts=onePerPart?parts.map((p,i)=>make(prefix+i,root?.id||(i?prefix+'0':null),[p.partId])):[make(root?.id||prefix+'0',null,parts.map(p=>p.partId))];
  return JSON.stringify({requestId,currentNodeId:upserts.at(-1).id,upserts});
}
function storage(initial){let value=initial;return {save:async next=>{value=structuredClone(next);},get value(){return structuredClone(value);}};}

test('compact API prompts preserve every original character and translate short references back to exact evidence',async()=>{
  const messages=Array.from({length:40},(_,i)=>msg('long-native-message-id-'+i,'不同尝试🚀'.repeat(800)));
  const history=await prepareHistory(messages);let sent='';
  const result=await organizeLong({messages,compact:true,save:async()=>{},run:async prompt=>{
    const {parts}=decode(prompt);sent+=parts.map(p=>p.text).join('');assert.ok(parts.every(p=>/^p\d+$/.test(p.partId)));return answer(prompt);
  }});
  assert.equal(sent,messages.map(m=>m.text).join(''));
  assert.deepEqual(Object.keys(result.published.done).sort(),history.parts.map(p=>p.partId).sort());
  assert.equal(new Set(result.published.nodes.flatMap(n=>n.sources)).size,40);
  assert.ok(result.published.nodes[0].evidence.every(e=>e.messageId.startsWith('long-native-message-id-')&&e.digest.length===64));
});

test('compact directory is bounded and short source aliases still reject invented evidence',async()=>{
  const history=await prepareHistory([msg('source','full text')]);
  const nodes=Array.from({length:60},(_,i)=>({id:'n'+i,parent:i?'n0':null,lane:i?'branch':'main',title:'title',summary:'s'.repeat(500),description:'d'.repeat(900),status:'待验证',sources:[]}));
  const state={nodes,currentNodeId:'n2',batchCount:4,done:{}};
  const regular=batchPrompt(history.parts,state,'req'),compact=batchPrompt(history.parts,state,'req',true);
  assert.ok(JSON.stringify(compact.catalog).length<12100);assert.ok(compact.prompt.length<regular.prompt.length*.5);
  const value=JSON.parse(answer(compact.prompt));value.upserts[0].sources=['p999'];
  assert.throws(()=>mergeBatch(JSON.stringify(value),state,history.parts,'req',compact.catalog,compact.prefix,compact.aliases),/不存在/);
});

test('compact pending batches keep their format on resume and automatic coverage repair uses short IDs',async()=>{
  const messages=[msg('source1','one'),msg('source2','two')],store=storage();let pendingId;
  await assert.rejects(organizeLong({messages,compact:true,save:store.save,run:async(prompt,id)=>{pendingId=id;throw Error('offline');}}),/offline/);
  let calls=0;
  const result=await organizeLong({messages,record:store.value,compact:false,save:store.save,run:async(prompt,id)=>{
    assert.equal(decode(prompt).parts[0].partId,'p1');const value=JSON.parse(answer(prompt));
    if(calls++===0){assert.equal(id,pendingId);value.upserts[0].sources=['p1'];}
    else {assert.match(prompt,/上次漏引的 partId：\["p2"\]/);assert.notEqual(id,pendingId);}
    return JSON.stringify(value);
  }});
  assert.equal(calls,2);assert.equal(result.job,null);assert.deepEqual(result.published.nodes[0].sources,['source1','source2']);
});

test('million-character message is covered through its tail without prompt-sized truncation',async()=>{
  const text=('正文🚀'.repeat(300000))+'最后的失败原因与转向',messages=[msg('long',text)];
  const history=await prepareHistory(messages);
  assert.equal(history.parts.map(p=>p.text).join(''),text);
  for(const p of history.parts){assert.ok(p.text.length<=6000);assert.ok(!/[\uD800-\uDBFF]$/.test(p.text));}
  const store=storage();let calls=0,maxPrompt=0;
  const result=await organizeLong({messages,save:store.save,run:async prompt=>{calls++;maxPrompt=Math.max(maxPrompt,prompt.length);return answer(prompt);}});
  assert.ok(calls>50);assert.ok(maxPrompt<30000);
  assert.equal(Object.keys(result.published.done).length,history.parts.length);
  assert.equal(result.published.nodes[0].evidence.at(-1).end,text.length);
  assert.deepEqual(result.published.incompleteSources,[]);assert.equal(store.value.job,null);
});

test('over 200 task nodes survive merging; later batches do not delete old branches',async()=>{
  const messages=Array.from({length:260},(_,i)=>msg('m'+i,'尝试'+i));
  const result=await organizeLong({messages,save:async()=>{},run:async prompt=>answer(prompt,{onePerPart:true})});
  assert.equal(result.published.nodes.length,260);
  assert.equal(new Set(result.published.nodes.flatMap(n=>n.sources)).size,260);
  const old=structuredClone(result.published.nodes);
  const incremental=await organizeLong({messages:[...messages,msg('new','下一步')],record:result,save:async()=>{},run:async prompt=>answer(prompt,{onePerPart:true})});
  assert.deepEqual(incremental.published.nodes.slice(0,260),old);assert.equal(incremental.published.nodes.length,261);
  let calls=0;await organizeLong({messages:[...messages,msg('new','下一步')],record:incremental,save:async()=>{},run:async()=>{calls++;}});assert.equal(calls,0);
});

test('pause persists exact pending batch; appended messages wait until the recovered batch commits',async()=>{
  const messages=Array.from({length:35},(_,i)=>msg('m'+i,'方案 '+i));
  const store=storage(),controller=new AbortController();let firstPending;
  await assert.rejects(organizeLong({messages,save:store.save,signal:controller.signal,run:async(prompt,id,session,save)=>{
    if(decode(prompt).prefix==='b2_'){firstPending=decode(prompt);session.turnId='accepted-turn';await save();controller.abort();controller.signal.throwIfAborted();}
    return answer(prompt,{onePerPart:true});
  }}),{name:'AbortError'});
  assert.equal(store.value.published.nodes.length,32);assert.equal(store.value.job.session.turnId,'accepted-turn');
  let calls=0;
  const resumed=await organizeLong({messages:[...messages,msg('appended','暂停期间追加的要求')],record:store.value,save:store.save,run:async(prompt,id,session)=>{
    if(calls++===0){assert.equal(id,firstPending.requestId);assert.deepEqual(decode(prompt).parts,firstPending.parts);assert.equal(session.turnId,'accepted-turn');}
    return answer(prompt,{onePerPart:true});
  }});
  assert.equal(calls,2);assert.equal(resumed.published.nodes.length,36);assert.equal(resumed.job,null);
});

test('partially processed long message stays visibly pending',async()=>{
  const messages=[msg('long','x'.repeat(40000))],store=storage();
  await assert.rejects(organizeLong({messages,save:store.save,run:async prompt=>{if(decode(prompt).prefix==='b2_')throw Error('offline');return answer(prompt);}}),/offline/);
  const graph=buildGraph(messages,store.value.published);
  assert.ok(graph.nodes.some(n=>n.id==='auto-long'&&n.status==='待整理'));
  const result=await organizeLong({messages,record:store.value,save:store.save,run:async p=>answer(p)});
  assert.ok(!buildGraph(messages,result.published).nodes.some(n=>n.id==='auto-long'));
});

test('edited history rebuild retains old published tree until the replacement completes',async()=>{
  const store=storage(),messages=[msg('m','old')];
  const original=await organizeLong({messages,save:store.save,run:async p=>answer(p)});
  await assert.rejects(organizeLong({messages:[msg('m','changed'.repeat(7000))],record:original,save:store.save,run:async p=>{if(decode(p).prefix==='b2_')throw Error('offline');return answer(p);}}),/offline/);
  assert.equal(store.value.job.kind,'rebuild');assert.deepEqual(store.value.published,original.published);
  const next=await organizeLong({messages:[msg('m','changed'.repeat(7000))],record:store.value,save:store.save,run:async p=>answer(p)});
  assert.notEqual(next.published.manifest.m,original.published.manifest.m);assert.equal(next.job,null);
});

test('invalid model coverage, injected sources and unknown nodes cannot advance a checkpoint',async()=>{
  const messages=[msg('m1','目标'),msg('m2','失败')],history=await prepareHistory(messages);
  const state={nodes:[],currentNodeId:null,batchCount:0,done:{}};
  const {catalog,prefix,prompt}=batchPrompt(history.parts,state,'request');
  for(const change of [v=>v.upserts[0].sources.pop(),v=>v.upserts[0].sources.push('invented'),v=>v.upserts[0].parent='missing',v=>v.upserts[0].id='old_steal']){
    const value=JSON.parse(answer(prompt));change(value);assert.throws(()=>mergeBatch(JSON.stringify(value),state,history.parts,'request',catalog,prefix));assert.deepEqual(state.done,{});
  }
  const store=storage();await assert.rejects(organizeLong({messages,save:store.save,run:async()=>'{bad result}'}),/格式无效/);
  assert.equal(store.value.published,null);assert.equal(store.value.job.pending,null);
});

test('failed durable save does not publish an in-memory result',async()=>{
  const store=storage();let published=0,calls=0;
  await assert.rejects(organizeLong({messages:[msg('m','任务')],save:async next=>{if(next.published)throw Error('disk full');await store.save(next);},onGraph:()=>published++,run:async p=>{calls++;return answer(p);}}),/disk full/);
  assert.equal(published,0);assert.equal(calls,1);assert.equal(store.value.published,null);assert.ok(store.value.job.pending);
  await organizeLong({messages:[msg('m','任务')],record:store.value,save:store.save,run:async p=>answer(p)});assert.equal(store.value.published.nodes.length,1);
});

test('missing coverage is corrected with exact feedback and a new durable request, without premature publication',async()=>{
  const messages=[msg('a','采用方案 A'),msg('b','方案 A 失败，转向 B')],store=storage();let calls=0,published=0,firstId;
  const result=await organizeLong({messages,save:store.save,onGraph:()=>published++,run:async(prompt,id)=>{
    const output=JSON.parse(answer(prompt));
    if(calls++===0){firstId=id;output.upserts[0].sources.pop();}
    else {assert.notEqual(id,firstId);assert.equal(published,0);assert.equal(store.value.job.state.batchCount,0);assert.match(prompt,/第 1\/2 次补正/);assert.ok(prompt.includes(store.value.job.pending.repair.missingPartIds[0]));assert.equal(decode(prompt).parts.length,2);}
    return JSON.stringify(output);
  }});
  assert.equal(calls,2);assert.equal(result.published.batchCount,1);assert.equal(Object.keys(result.published.done).length,2);
});

test('coverage correction is bounded and resumes the same pending correction after interruption',async()=>{
  const messages=[msg('a','目标'),msg('b','尝试')],store=storage();let calls=0,repairId;
  await assert.rejects(organizeLong({messages,save:store.save,run:async(prompt,id)=>{
    if(calls++===1){repairId=id;throw Error('offline');}
    const output=JSON.parse(answer(prompt));output.upserts[0].sources.pop();return JSON.stringify(output);
  }}),/offline/);
  assert.equal(store.value.job.pending.repair.attempt,1);
  let retries=0;
  await assert.rejects(organizeLong({messages,record:store.value,save:store.save,run:async(prompt,id)=>{
    if(retries++===0)assert.equal(id,repairId);
    const output=JSON.parse(answer(prompt));output.upserts[0].sources.pop();return JSON.stringify(output);
  }}),/未归入任务树/);
  assert.equal(retries,2);assert.equal(store.value.published,null);assert.equal(store.value.job.state.batchCount,0);assert.equal(store.value.job.pending,null);
});

test('active assistant answers are deferred until the native turn completes',async()=>{
  const message={id:'a',text:'streaming',role:'assistant',phase:'final_answer',turnStatus:'inProgress'};
  assert.equal((await prepareHistory([message])).parts.length,0);
  message.turnStatus='completed';assert.equal((await prepareHistory([message])).parts.length,1);
});

test('deep 10000-node trees render a bounded page without recursive stack overflow',()=>{
  const nodes=Array.from({length:10000},(_,i)=>({id:'n'+i,parent:i?'n'+(i-1):null,title:'任务',summary:'摘要',description:'详情',lane:i?'branch':'main',status:'已确认',sources:['m']}));
  assert.equal(visibleTreeRows(nodes,'n9999').rows.length,10000);
  const page=treeMarkup(nodes,'n9999',new Set(),null,{offset:9900});
  assert.equal((page.match(/data-node=/g)||[]).length,100);assert.match(page,/data-node="n9999"/);
  const collapsed=treeMarkup(nodes,'n9999',new Set(['n0']));assert.equal((collapsed.match(/data-node=/g)||[]).length,1);
});

test('transcript pagination includes last message and reconstructs long Unicode content without loss',()=>{
  const text='x'.repeat(11999)+'🚀'+('内容'.repeat(15000))+'末尾',messages=Array.from({length:101},(_,i)=>msg('m'+i,i===100?text:'消息'+i));
  const last=transcriptPage(messages,{page:5});assert.match(last.html,/source-m100/);assert.match(last.html,/分段查看完整消息/);
  const first=transcriptPage(messages,{focusId:'m100'});let reconstructed='';
  for(let part=0;part<first.total;part++){const page=transcriptPage(messages,{focusId:'m100',part});const raw=page.html.match(/<pre>([\s\S]*?)<\/pre>/)[1];assert.ok(!/[\uD800-\uDBFF]$/.test(raw));reconstructed+=raw;}
  assert.equal(reconstructed,text);assert.equal((transcriptPage(messages).html.match(/<article/g)||[]).length,20);
});
