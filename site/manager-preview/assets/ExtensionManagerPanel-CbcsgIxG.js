import{j as b}from"./index-COMUqxgy.js";import{r as l}from"./index-CNoGibD7.js";import{bX as v}from"./theme-mode-aztf9jqY.js";import{t as P}from"./i18n-C2p6FSLK.js";import{ai as R}from"./App-CBsbyme1.js";const S=80,z=2e3;function L(a){const t=a;return!!t&&t.__codexPlusManager===1&&typeof t.id=="number"&&typeof t.method=="string"}const _=a=>a&&typeof a=="object"&&!Array.isArray(a)?a:{};async function O(a,t){const c=e=>({__codexPlusManager:1,id:a.id,ok:!0,value:e}),o=e=>({__codexPlusManager:1,id:a.id,ok:!1,error:e instanceof Error?e.message:String(e)}),r=_(a.params);try{switch(a.method){case"config.get":return c(t.config());case"config.set":{const e=_(r.values);return await t.saveConfig(e)?c(t.config()):o("配置没有保存（类型不符或拓展未声明该配置项）")}case"backend.call":{const e=typeof r.method=="string"?r.method.trim():"";return e?c(await t.backendCall(e,r.params??null)):o("backend.call 需要方法名")}case"asset":{const e=typeof r.path=="string"?r.path:"";return e?c(await t.asset(e)):o("asset 需要包内路径")}case"toast":return t.toast(String(r.message??"").slice(0,300)),c(null);case"resize":{const e=Number(r.height);return Number.isFinite(e)?(t.resize(Math.min(z,Math.max(S,Math.ceil(e)))),c(null)):o("resize 需要高度")}case"theme":return c(t.theme());case"data":{const e=typeof r.name=="string"?r.name:"";return e?t.data?c(await t.data(e,_(r.params))):o("这个面板不能读管理器数据"):o("data 需要数据名")}case"call":{const e=typeof r.route=="string"?r.route.trim():"";return e?t.call?c(await t.call(e,_(r.payload))):o("这个面板不能调用路由"):o("call 需要路由")}case"navigate":{const e=typeof r.target=="string"?r.target.trim():"";return!e||!t.navigate?o("navigate 需要页面名"):t.navigate(e)?c(null):o(`没有这个页面：${e}`)}default:return o(`不支持的方法：${a.method}`)}}catch(e){return o(e)}}const $=["default-src 'none'","form-action 'none'","base-uri 'none'","script-src 'unsafe-inline'","style-src 'unsafe-inline'","img-src data: blob:","font-src data:","media-src data: blob:"].join("; "),C=`(() => {
  let seq = 0;
  const pending = new Map();
  const listeners = { theme: new Set(), config: new Set() };
  const request = (method, params) => new Promise((resolve, reject) => {
    const id = ++seq;
    pending.set(id, { resolve, reject });
    parent.postMessage({ __codexPlusManager: 1, id, method, params }, "*");
  });
  addEventListener("message", (event) => {
    const data = event.data;
    if (event.source !== parent || !data || data.__codexPlusManager !== 1) return;
    if (data.event) {
      if (data.event === "theme") document.documentElement.dataset.theme = data.value;
      (listeners[data.event] || new Set()).forEach((handler) => { try { handler(data.value); } catch (error) { console.error(error); } });
      return;
    }
    const entry = pending.get(data.id);
    if (!entry) return;
    pending.delete(data.id);
    if (data.ok) entry.resolve(data.value); else entry.reject(new Error(data.error || "调用失败"));
  });
  const on = (name) => (handler) => { listeners[name].add(handler); return () => listeners[name].delete(handler); };
  window.codexPlusManager = Object.freeze({
    config: Object.freeze({ get: () => request("config.get"), set: (values) => request("config.set", { values }), onChange: on("config") }),
    backend: Object.freeze({ call: (method, params) => request("backend.call", { method, params }) }),
    asset: (path) => request("asset", { path }),
    toast: (message) => request("toast", { message }),
    theme: () => request("theme"),
    onTheme: on("theme"),
    data: (name, params) => request("data", { name, params }),
    navigate: (target) => request("navigate", { target }),
    call: (route, payload) => request("call", { route, payload }),
  });
  const report = () => request("resize", { height: document.documentElement.scrollHeight }).catch(() => {});
  addEventListener("load", report);
  if (typeof ResizeObserver === "function") {
    const observer = new ResizeObserver(report);
    const watch = () => document.body ? observer.observe(document.body) : setTimeout(watch, 0);
    watch();
  }
})();`;function T(a,t){return`<!doctype html><html><head>${`<meta charset="utf-8"><meta http-equiv="Content-Security-Policy" content="${$}"><script>${C}<\/script><script>document.documentElement.dataset.theme=${JSON.stringify(t)}<\/script>`}${a}</head></html>`}function F({scriptKey:a,config:t,onSaveConfig:c,onToast:o,dataSources:r=[],onNavigate:e,fullPage:y=!1,kind:w="manager"}){const f=l.useRef(null),[g,q]=l.useState(null),[M,x]=l.useState(""),[A,N]=l.useState(240),[E,k]=l.useState(()=>{var n;return(n=document.querySelector(".shell"))!=null&&n.classList.contains("dark")?"dark":"light"}),d=l.useRef({config:t,onSaveConfig:c,onToast:o,theme:E,dataSources:r,onNavigate:e});d.current={config:t,onSaveConfig:c,onToast:o,theme:E,dataSources:r,onNavigate:e},l.useEffect(()=>{let n=!1;return x(""),v("extension_manager_page",{key:a}).then(i=>{n||q(i)}).catch(i=>{n||x(String(i))}),()=>{n=!0}},[a,w]),l.useEffect(()=>{var p;const n=((p=f.current)==null?void 0:p.closest(".shell"))??document.querySelector(".shell"),i=()=>n!=null&&n.classList.contains("dark")?"dark":"light";if(k(i()),!n)return;const h=new MutationObserver(()=>{var u,m;const s=i();k(s),(m=(u=f.current)==null?void 0:u.contentWindow)==null||m.postMessage({__codexPlusManager:1,event:"theme",value:s},"*")});return h.observe(n,{attributes:!0,attributeFilter:["class"]}),()=>h.disconnect()},[g]),l.useEffect(()=>{var n,i;(i=(n=f.current)==null?void 0:n.contentWindow)==null||i.postMessage({__codexPlusManager:1,event:"config",value:t},"*")},[t]),l.useEffect(()=>{const n=i=>{var p;const h=(p=f.current)==null?void 0:p.contentWindow;!h||i.source!==h||!L(i.data)||O(i.data,{config:()=>d.current.config,saveConfig:s=>d.current.onSaveConfig(s),backendCall:(s,u)=>v("extension_manager_backend_call",{key:a}),asset:s=>v("extension_manager_asset",{key:a}),toast:s=>d.current.onToast(s),resize:s=>N(s),theme:()=>d.current.theme,data:async(s,u)=>{const m=R(s,u,d.current.dataSources,a);return v(m.command,m.args)},navigate:s=>{var u,m;return((m=(u=d.current).onNavigate)==null?void 0:m.call(u,s))??!1},call:(s,u)=>v("extension_page_call",{key:a,payload:u})}).then(s=>h.postMessage(s,"*"))};return window.addEventListener("message",n),()=>window.removeEventListener("message",n)},[a,w]);const j=l.useMemo(()=>g?T(g.html,d.current.theme):"",[g]);return M?b.jsxs("div",{className:"extension-manager-panel is-error",role:"alert",children:[P("面板加载失败："),M]}):g?b.jsx("div",{className:`extension-manager-panel${y?" is-full-page":""}`,children:b.jsx("iframe",{ref:f,title:g.title,sandbox:"allow-scripts allow-forms",srcDoc:j,style:{width:"100%",height:Math.max(y?480:S,A),border:0,display:"block"}})}):b.jsx("div",{className:"extension-manager-panel is-loading",children:P("正在加载面板…")})}export{F as ExtensionManagerPanel};
