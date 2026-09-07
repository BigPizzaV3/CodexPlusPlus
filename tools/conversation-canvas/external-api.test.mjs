import test from 'node:test';
import assert from 'node:assert/strict';
import {apiEndpoint,apiConfig,storedApiConfig,completionText,apiError,createApiOrganizer,apiRequestBody,readApiResponse} from './external-api.mjs';
import {organizeLong} from './long-organizer.mjs';

const config=()=>apiConfig({channel:'external',baseUrl:'https://provider.example/v1',model:'example-model',key:'secret-test-key',revision:'fixture'});
test('DeepSeek fast mode disables thinking only for the official V4 endpoint, with a persistent opt-out',()=>{
  const deep=apiConfig({...config(),baseUrl:'https://api.deepseek.com',model:'deepseek-v4-flash'});
  assert.deepEqual(apiRequestBody(deep,'p').thinking,{type:'disabled'});
  assert.equal(apiRequestBody({...deep,speed:'provider'},'p').thinking,undefined);
  assert.equal(apiRequestBody({...deep,endpoint:'https://provider.example/v1/chat/completions'},'p').thinking,undefined);
  assert.equal(apiRequestBody({...deep,model:'future-model'},'p').thinking,undefined);
  assert.equal(storedApiConfig({...deep,speed:'provider'}).speed,'provider');
});

function streamResponse(text,headers={'Content-Type':'text/event-stream'}){
  const bytes=new TextEncoder().encode(text);let at=0;
  return new Response(new ReadableStream({pull(controller){if(at===bytes.length){controller.close();return;}controller.enqueue(bytes.slice(at,at+=Math.min(7,bytes.length-at)));}}),{headers});
}
test('SSE handles split UTF-8, keepalives and DONE without exposing reasoning; JSON fallback works',async()=>{
  const progress=[];
  const payload=': keepalive\r\n\r\ndata: '+JSON.stringify({choices:[{delta:{reasoning_content:'private reasoning'}}]})+'\n\n'+
    'data: '+JSON.stringify({choices:[{delta:{content:'树形🚀'}}]})+'\n\n'+
    'data: '+JSON.stringify({choices:[{delta:{},finish_reason:'stop'}]})+'\n\ndata: [DONE]\n\n';
  const result=await readApiResponse(streamResponse(payload),undefined,p=>progress.push(p));
  assert.equal(completionText(result),'树形🚀');assert.ok(progress.some(p=>p.chars===4));
  assert.doesNotMatch(JSON.stringify(result)+JSON.stringify(progress),/private reasoning/);
  const fallback={choices:[{message:{content:'ok'}}]};
  assert.deepEqual(await readApiResponse(streamResponse('\n '+JSON.stringify(fallback),{'Content-Type':'application/json'})),fallback);
});
test('truncated streams, HTTP failures and output truncation cannot become a completed tree',async()=>{
  await assert.rejects(readApiResponse(streamResponse('data: {"choices":[{"delta":{"content":"partial"}}]}\n\n')),/提前结束/);
  await assert.rejects(readApiResponse(new Response('',{status:429})),e=>e.status===429);
  const result=await readApiResponse(streamResponse('data: {"choices":[{"delta":{"content":"partial"},"finish_reason":"length"}]}\n\ndata: [DONE]\n'));
  assert.throws(()=>completionText(result),/长度上限/);
});
test('one click retries temporary failures and stops after the retry budget or a permanent error',async()=>{
  let calls=0;const progress=[];
  const runner=createApiOrganizer({retryDelayMs:1,request:async()=>{if(++calls<3)throw {status:503};return {choices:[{message:{content:'ok'}}]};}});
  assert.equal(await runner.run(config(),'p','id',{},async()=>{},undefined,t=>progress.push(t)),'ok');assert.equal(calls,3);
  assert.ok(progress.some(p=>p.includes('自动重试')));runner.dispose();
  for(const [status,expected] of [[429,3],[401,1]]){
    let count=0;const failed=createApiOrganizer({retryDelayMs:1,request:async()=>{count++;throw {status};}});
    await assert.rejects(failed.run(config(),'p','id',{},async()=>{}));assert.equal(count,expected);failed.dispose();
  }
});
test('pause during automatic retry backoff does not submit another request',async()=>{
  let calls=0;const abort=new AbortController();
  const runner=createApiOrganizer({retryDelayMs:10000,request:async()=>{calls++;throw {status:503};}});
  await assert.rejects(runner.run(config(),'p','id',{},async()=>{},abort.signal,t=>{if(t.includes('自动重试'))abort.abort();}),{name:'AbortError'});
  assert.equal(calls,1);runner.dispose();
});
test('dispose is cancellation rather than timeout and a non-cooperating request still times out',async()=>{
  let began;const started=new Promise(r=>{began=r;});
  const runner=createApiOrganizer({request:async()=>{began();return new Promise(()=>{});}});
  const work=runner.run(config(),'p','id',{},async()=>{});await started;runner.dispose();await assert.rejects(work,{name:'AbortError'});
  const timeout=createApiOrganizer({timeoutMs:5,maxRetries:0,request:async()=>new Promise(()=>{})});
  await assert.rejects(timeout.run(config(),'p','id',{},async()=>{}),/请求超时/);timeout.dispose();
});
test('API endpoints accept base or full Chat Completions URL and reject credentials',()=>{
  for(const url of ['https://provider.example','https://provider.example/v1/','https://provider.example/v1/chat/completions'])assert.equal(apiEndpoint(url),'https://provider.example/v1/chat/completions');
  assert.equal(apiEndpoint('https://provider.example/custom/v2'),'https://provider.example/custom/v2/chat/completions');
  for(const url of ['http://provider.example','https://key@provider.example','https://provider.example?key=secret','file:///tmp/api'])assert.throws(()=>apiEndpoint(url));
});
test('credentials require explicit remember and are absent from ordinary stored config',()=>{
  const value=config();assert.equal(storedApiConfig(value).key,undefined);
  assert.equal(storedApiConfig({...value,remember:true}).key,value.key);
  assert.throws(()=>apiConfig({...value,key:''}),/API Key/);
});
test('API errors do not expose echoed credential or request content',()=>{
  assert.equal(apiError({status:401,message:'secret-test-key full private prompt'}).message,'API 401：密钥无效或已过期');
  assert.doesNotMatch(apiError({message:'secret-test-key'}).message,/secret-test-key/);
  assert.throws(()=>completionText({choices:[{finish_reason:'length',message:{content:'partial'}}]}),/长度上限/);
  assert.throws(()=>completionText({choices:[]}),/Chat Completions/);
  assert.equal(completionText({choices:[{message:{content:[{type:'text',text:'ok'}]}}]}),'ok');
});
test('paused API request resumes once, does not persist secret, changing profile isolates results',async()=>{
  let calls=0,finish;const checkpoints=[];
  const runner=createApiOrganizer({maxRetries:0,request:async(url,options)=>{
    calls++;assert.equal(url,'https://provider.example/v1/chat/completions');assert.equal(options.headers.Authorization,'Bearer secret-test-key');
    assert.deepEqual(JSON.parse(options.body),{model:'example-model',stream:true,messages:[{role:'user',content:'synthetic prompt'}]});
    return new Promise(resolve=>{finish=()=>resolve({choices:[{message:{content:'ok'}}]});});
  }});
  const session={sideId:'old-native',profile:'native'},abort=new AbortController();
  const save=async()=>checkpoints.push(structuredClone(session));
  const first=runner.run(config(),'synthetic prompt','request-one',session,save,abort.signal);
  await new Promise(resolve=>setTimeout(resolve,0));abort.abort();await assert.rejects(first,{name:'AbortError'});finish();
  assert.equal(await runner.run(config(),'synthetic prompt','request-one',session,save),'ok');assert.equal(calls,1);
  assert.doesNotMatch(JSON.stringify(checkpoints),/secret-test-key|old-native|synthetic prompt/);
  const next=runner.run({...config(),revision:'new'},'synthetic prompt','request-one',session,save);await new Promise(resolve=>setTimeout(resolve,0));finish();await next;assert.equal(calls,2);runner.dispose();
});
test('failed API requests can be retried; timeout releases the request',async()=>{
  let calls=0;const runner=createApiOrganizer({maxRetries:0,request:async()=>{if(!calls++)throw {status:429,message:'secret'};return {choices:[{message:{content:'ok'}}]};}});
  const session={};await assert.rejects(runner.run(config(),'test','one',session,async()=>{}),/429/);
  assert.equal(await runner.run(config(),'test','one',session,async()=>{}),'ok');runner.dispose();
  const timeout=createApiOrganizer({timeoutMs:5,maxRetries:0,request:async(url,{signal})=>new Promise((resolve,reject)=>signal.addEventListener('abort',()=>reject(signal.reason),{once:true}))});
  await assert.rejects(timeout.run(config(),'test','one',{},async()=>{}),/超时/);timeout.dispose();
});
test('external API batches preserve all message coverage and resume after 429',async()=>{
  let record,fail=true,calls=0;
  const runner=createApiOrganizer({maxRetries:0,request:async(url,options)=>{
    calls++;if(calls===2&&fail){fail=false;throw {status:429};}
    const prompt=JSON.parse(options.body).messages[0].content;
    const requestId=prompt.match(/"requestId":"([^"]+)"/)[1],prefix=prompt.match(/新 ID 必须以 (b[0-9]+_) 开头/)[1];
    const catalog=JSON.parse(prompt.split('已有节点目录（摘要可能缩短，以原 ID 为准）：')[1].split('\n')[0]);
    const parts=JSON.parse(prompt.split('本批资料：')[1].split('\n')[0]);
    const root=catalog.find(n=>n.parent===null);
    return {choices:[{message:{content:JSON.stringify({requestId,currentNodeId:root?.id||prefix+'0',upserts:parts.map((part,i)=>({id:prefix+i,parent:root?.id||(i?prefix+'0':null),lane:i||root?'branch':'main',title:'Task',summary:'Summary',description:'Details',status:'待验证',sources:[part.partId]}))})}}]};
  }});
  const messages=Array.from({length:20},(_,i)=>({id:'m'+i,role:'user',text:'长对话内容'.repeat(800)}));
  const options={messages,save:async value=>{record=structuredClone(value);},run:(p,id,session,save)=>runner.run(config(),p,id,session,save)};
  await assert.rejects(organizeLong(options),/429/);assert.equal(record.published.batchCount,1);
  const result=await organizeLong({...options,record});assert.equal(result.job,null);
  assert.equal(new Set(result.published.nodes.flatMap(n=>n.sources)).size,20);
  assert.doesNotMatch(JSON.stringify(result),/secret-test-key/);runner.dispose();
});
