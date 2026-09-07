// Verified Desktop 26.901.6511 adapter: HTTP uses the app host, not renderer fetch/CSP.
async function nativeApiRequest(url,options){
  const localError=message=>Object.assign(Error(message),{canvasApiLocal:true});
  let native;
  try{native=await import('app://-/assets/app-initial-f87238153a19.js');}catch{throw localError('当前 Codex 版本的 API 适配器不兼容，请更新对话脉络脚本');}
  if(typeof native.lGt!=='function')throw localError('当前 Codex 版本未提供兼容的 HTTP 服务');
  native.lGt();
  const client=native.cGt?.getInstance?.();
  if(typeof client?.fetch!=='function'||!native.jR?.httpFetch)throw localError('当前 Codex 的 HTTP 服务尚未就绪，请稍后重试');
  const {onProgress,...fetchOptions}=options;
  const response=await client.fetch(url,fetchOptions);
  return readApiResponse(response,options.signal,onProgress);
}
