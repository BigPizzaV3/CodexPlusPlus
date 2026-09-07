// OpenAI-compatible Chat Completions. Never persist credentials in tree checkpoints.
export function apiEndpoint(raw){
  let url;try{url=new URL(String(raw).trim());}catch{throw Error('请输入完整的 HTTPS API 地址');}
  if(url.protocol!=='https:'||url.username||url.password||url.search||url.hash)throw Error('API 地址须使用 HTTPS，且不含账号、密码、查询参数或片段');
  const path=url.pathname.replace(/\/+$/,'');
  url.pathname=path.endsWith('/chat/completions')?path:(path||'/v1')+'/chat/completions';
  return url.href;
}

export function apiConfig(input){
  const channel=input?.channel==='external'?'external':'native';
  const value={channel,baseUrl:String(input?.baseUrl||'').trim(),model:String(input?.model||'').trim(),key:String(input?.key||'').trim(),remember:input?.remember===true,speed:input?.speed==='provider'?'provider':'fast',revision:input?.revision||crypto.randomUUID()};
  if(channel==='external'){
    value.endpoint=apiEndpoint(value.baseUrl);
    if(!value.model||value.model.length>200)throw Error('请填写 API 的模型名称');
    if(!value.key||/[\r\n]/.test(value.key))throw Error('请在设置中填写有效 API Key');
  }
  return value;
}

export function storedApiConfig(config){
  const {channel,baseUrl,model,remember,speed,revision}=config;
  return {channel,baseUrl,model,remember,speed,revision,...remember?{key:config.key}:{}};
}

export function apiError(error){
  if(error?.name==='AbortError')return error;
  if(error?.canvasApiLocal===true)return error;
  const code=Number(error?.status??error?.responseStatus);
  const hint={401:'密钥无效或已过期',403:'接口拒绝访问，请检查权限',404:'地址或模型不存在',408:'接口请求超时',413:'本批资料超过接口大小限制',429:'接口限流或额度不足'}[code];
  // Provider messages can echo request contents and Authorization; never display them.
  return Object.assign(Error(hint?`API ${code}：${hint}`:code>=400?`API 请求失败（HTTP ${code}），请检查服务状态`:'API 连接失败，请检查地址、网络及服务状态'),{retryable:!code||code===408||code===429||code>=500});
}

export function apiRequestBody(config,prompt){
  const body={model:config.model,stream:true,messages:[{role:'user',content:prompt}]};
  // Only send provider-specific options to the documented official endpoint.
  if(new URL(config.endpoint).hostname==='api.deepseek.com'&&/^deepseek-v4-(flash|pro)$/.test(config.model)&&config.speed!=='provider')body.thinking={type:'disabled'};
  return body;
}

// Native HTTP returns a ReadableStream. Decode SSE incrementally without ever
// publishing a partial tree or retaining the provider's reasoning text.
export async function readApiResponse(response,signal,progress=()=>{}){
  if(response.status>=400)throw {status:response.status};
  const reader=response.body?.getReader();
  if(!reader)throw Object.assign(Error('API 响应为空'),{canvasApiLocal:true});
  const decoder=new TextDecoder();let buffer='',raw='',size=0,content='',finish=null,done=false,sse=/text\/event-stream/i.test(response.headers?.get?.('content-type')||'');
  const consume=line=>{
    if(!line.startsWith('data:'))return;
    const data=line.slice(5).trim();if(!data)return;if(data==='[DONE]'){done=true;return;}
    let value;try{value=JSON.parse(data);}catch{throw Object.assign(Error('API 流式数据格式无效，本批未提交'),{canvasApiLocal:true,retryable:true});}
    if(value.error)throw Object.assign(Error('API 流式生成中断，本批未提交'),{canvasApiLocal:true,retryable:true});
    const choice=value.choices?.[0];if(choice?.delta?.content)content+=choice.delta.content;
    if(choice?.finish_reason)finish=choice.finish_reason;
    if(choice?.delta?.refusal)throw Object.assign(Error('API 未能生成本批整理结果'),{canvasApiLocal:true});
  };
  try{
    for(;;){
      const part=await waitForApi(reader.read(),signal);if(part.done)break;
      size+=part.value.byteLength;if(size>4*1024*1024)throw Object.assign(Error('API 响应超过大小限制'),{canvasApiLocal:true});
      const text=decoder.decode(part.value,{stream:true});
      if(!sse){raw+=text;if(/^\s*(data:|:)/.test(raw)){sse=true;buffer=raw;raw='';}}
      else buffer+=text;
      if(sse){let end;while((end=buffer.indexOf('\n'))>=0){consume(buffer.slice(0,end).replace(/\r$/,''));buffer=buffer.slice(end+1);}}
      progress({bytes:size,chars:content.length});if(done)break;
    }
    const tail=decoder.decode();if(sse){buffer+=tail;if(buffer.trim())consume(buffer.replace(/\r$/,''));
      if(!done&&!finish)throw Object.assign(Error('API 流式连接提前结束，本批未提交'),{canvasApiLocal:true,retryable:true});
      return {choices:[{finish_reason:finish,message:{content}}]};
    }
    try{return JSON.parse(raw+tail);}catch{throw Object.assign(Error('API 返回的不是 JSON，请检查 API 地址'),{canvasApiLocal:true});}
  }finally{void reader.cancel().catch(()=>{});try{reader.releaseLock();}catch{}}
}

export function completionText(body){
  const choice=body?.choices?.[0];
  if(choice?.finish_reason==='length')throw Error('API 输出达到长度上限，本批未提交；请使用输出容量更大的模型');
  if(choice?.finish_reason==='content_filter'||choice?.message?.refusal)throw Error('API 未能生成本批整理结果');
  const content=choice?.message?.content;
  const value=typeof content==='string'?content:Array.isArray(content)?content.filter(p=>p?.type==='text').map(p=>p.text||'').join(''):'';
  if(!value.trim())throw Error('API 未返回有效的 choices[0].message.content，请确认兼容 Chat Completions');
  return value;
}

export function waitForApi(promise,signal){
  signal?.throwIfAborted();
  if(!signal)return promise;
  return new Promise((resolve,reject)=>{
    const abort=()=>{signal.removeEventListener('abort',abort);reject(signal.reason||new DOMException('Aborted','AbortError'));};
    signal.addEventListener('abort',abort,{once:true});
    promise.then(value=>{signal.removeEventListener('abort',abort);resolve(value);},error=>{signal.removeEventListener('abort',abort);reject(error);});
  });
}

export function createApiOrganizer({request,timeoutMs=300000,maxRetries=2,retryDelayMs=2000}){
  const requests=new Map();
  const closed=()=>new DOMException('整理连接已关闭','AbortError');
  const dispose=()=>{for(const entry of requests.values())entry.controller.abort(closed());requests.clear();};
  async function run(config,prompt,requestId,session,onSession,signal,progress=()=>{}){
    signal?.throwIfAborted();
    const profile=`external:${config.revision}`,slot=profile,key=`${profile}:${requestId}`;
    if(session.profile!==profile){for(const name of Object.keys(session))delete session[name];session.profile=profile;}
    for(let attempt=0;;attempt++){
      signal?.throwIfAborted();
      let entry=requests.get(slot);
      if(entry?.key!==key){
        entry?.controller.abort(closed());
        const resumed=session.requestId===requestId&&session.phase==='submitted';
        session.requestId=requestId;session.phase='submitted';await onSession();signal?.throwIfAborted();
        const controller=new AbortController(),started=Date.now();
        entry={key,controller,promise:null,chars:0,bytes:0,started};
        const activeEntry=entry;
        const timer=setTimeout(()=>controller.abort(Object.assign(Error(`API 请求超时（${Math.round(timeoutMs/1000)} 秒），本批未提交`),{canvasApiLocal:true,retryable:true})),timeoutMs);
        entry.promise=(async()=>{
          let response;
          try{
            response=await waitForApi(request(config.endpoint,{method:'POST',headers:{'Content-Type':'application/json',Authorization:`Bearer ${config.key}`},body:JSON.stringify(apiRequestBody(config,prompt)),signal:controller.signal,onProgress:info=>Object.assign(activeEntry,info)}),controller.signal);
          }catch(error){if(controller.signal.aborted)throw controller.signal.reason;throw apiError(error);}
          return completionText(response);
        })().finally(()=>clearTimeout(timer));
        entry.promise.catch(()=>{if(requests.get(slot)===activeEntry)requests.delete(slot);});
        requests.set(slot,entry);
        progress(resumed?'重新发送未收到结果的批次（接口可能重复计费）…':`正在通过 ${config.model} 整理…`);
      }
      const report=()=>progress(`${config.model} · ${entry.chars?`已生成 ${entry.chars} 字符`:entry.bytes?'服务端已连接，等待结果':'等待 API 响应'} · 已用 ${Math.floor((Date.now()-entry.started)/1000)} 秒${attempt?` · 重试 ${attempt}/${maxRetries}`:''}`);
      report();const heartbeat=setInterval(report,1000);
      try{return await waitForApi(entry.promise,signal);}
      catch(error){
        clearInterval(heartbeat);
        signal?.throwIfAborted();
        if(!error.retryable||attempt>=maxRetries)throw error;
        const delay=retryDelayMs*2**attempt;
        progress(`${error.message}；${Math.ceil(delay/1000)} 秒后自动重试 ${attempt+1}/${maxRetries}（可能重复计费）…`);
        // Pausing during backoff must not submit another request.
        let timer;try{await waitForApi(new Promise(resolve=>{timer=setTimeout(resolve,delay);}),signal);}finally{clearTimeout(timer);}
      }finally{clearInterval(heartbeat);}
    }
  }
  return {run,dispose};
}
