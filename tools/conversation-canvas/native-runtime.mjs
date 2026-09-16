// Private Desktop exports change between builds. Only bind reviewed contracts;
// discovering a new asset does not make its minified export names compatible.
export const nativeBuilds={
  'app-initial-f87238153a19.js':{ESt:'ESt',kr:'kr',Mr:'Mr',Q1:'Q1',lGt:'lGt',cGt:'cGt',jR:'jR'},
  'app-initial-bcc2ff475eb6.js':{ESt:'EDt',kr:'MDt',lGt:'hJt',cGt:'mJt',jR:'TW'},
};
export function nativeAssetCandidates(doc=globalThis.document,perf=globalThis.performance){
  const urls=[...Array.from(doc?.querySelectorAll?.('script[src],link[rel="modulepreload"][href]')||[],e=>e.src||e.href),
    ...(perf?.getEntriesByType?.('resource')||[]).map(e=>e.name)];
  return [...new Set(urls.filter(url=>/^app:\/\/-\/assets\/app-initial-[\w-]+\.js$/.test(url)))];
}
export function bindNativeRuntime(module,filename){
  const mapping=nativeBuilds[filename];
  if(!mapping)throw Error('当前 Codex 版本尚未适配对话脉络，请更新插件；已有整理结果仍保留');
  const adapter={};
  // Keep live bindings: initialization populates the HTTP class and services.
  for(const [key,value] of Object.entries(mapping))Object.defineProperty(adapter,key,{get:()=>module[value]});
  return adapter;
}
export function createNativeLoader({discover=nativeAssetCandidates,importModule=url=>import(url)}={}){
  let pending;
  return function load(){
    if(!pending)pending=(async()=>{
      const discovered=discover();
      // Without resource timing (e.g. after a reload), try reviewed builds only.
      const candidates=discovered.length?discovered:Object.keys(nativeBuilds).reverse().map(name=>'app://-/assets/'+name);
      for(const url of candidates){
        const filename=url.split('/').pop();
        if(!nativeBuilds[filename])continue;
        let module;
        try{module=await importModule(url);}catch{continue;}
        return bindNativeRuntime(module,filename);
      }
      throw Error(discovered.some(url=>!nativeBuilds[url.split('/').pop()])
        ?'当前 Codex 版本尚未适配对话脉络，请更新插件；已有整理结果仍保留'
        :'Codex 原生模块加载失败，请重新加载应用后重试；已有整理结果仍保留');
    })().catch(error=>{pending=null;throw error;});
    return pending;
  };
}
export const loadNativeRuntime=createNativeLoader();
