// Isolated fixture using the Codex++ export contract, with both frame and network access blocked.
import http from 'node:http';
import fs from 'node:fs';
const root=new URL('../public/',import.meta.url);
const port=Number(process.env.CANVAS_HARNESS_PORT||47834),id='11111111-1111-1111-1111-111111111111';
http.createServer((req,res)=>{
  const u=new URL(req.url,`http://127.0.0.1:${port}`);
  res.setHeader('Cache-Control','no-store');
  res.setHeader('Content-Security-Policy',"default-src 'none'; script-src 'self'; style-src 'unsafe-inline'; frame-src 'none'; connect-src 'none'");
  if(u.pathname==='/native-fixture.js'){
    res.setHeader('Content-Type','text/javascript;charset=utf-8');
    res.end(`
      const parent={cwd:'D:/fixture',latestCollaborationMode:null};
      const sides=new Map();let sideCount=0,turnCount=0,apiCalls=0;
      const manager={getHostId:()=> 'local',getConversation:id=>id==='${id}'?parent:sides.get(id),sendRequest:async(method,params)=>{
        if(method==='thread/turns/list')return {data:[{id:'history-turn',status:'completed'}]};
        if(method==='thread/items/list')return {data:[{turnId:'history-turn',item:{id:'m1',type:'userMessage',content:[{type:'text',text:'验证连接失败后重新点击能够恢复画布。'}]}}]};
        throw Error('Unexpected native read');
      }};
      const scope={get(){},value:{routeKind:'local-thread',conversationId:'${id}'}};
      const create=async()=>{throw Error('failed to prepare paginated fork: expected ordinal 638, got 637');};
      const createFiber={memoizedProps:{onCreateSideConversation:create}};
      const routeFiber={memoizedState:{memoizedState:{current:scope}},child:createFiber};createFiber.return=routeFiber;
      const rootFiber={child:routeFiber};routeFiber.return=rootFiber;
      const main=document.querySelector('main');main.__reactContainer$fixture={stateNode:{current:rootFiber}};
      if(!new URLSearchParams(location.search).has('noeditor')){
        const wrapper=document.createElement('div');wrapper.__reactFiber$fixture={return:createFiber};
        const editor=document.createElement('div');editor.contentEditable='true';editor.setAttribute('aria-label','主对话草稿');editor.textContent='主对话草稿保持原样';
        wrapper.append(editor);main.append(wrapper);
      }
      export function kr(){};
      export function lGt(){};
      export const jR={httpFetch:{}};
      export const cGt={getInstance:()=>({fetch:async(url,options)=>{
        const body=JSON.parse(options.body),prompt=body.messages[0].content;
        if(options.headers.Authorization!=='Bearer fixture-key')throw {status:401};
        let content='OK';
        if(prompt!=='Reply with OK only.'){
          apiCalls++;document.getElementById('status').textContent='API 整理请求次数：'+apiCalls;
          if(new URLSearchParams(location.search).has('apiretry')&&apiCalls===1)throw {status:503};
          const requestId=prompt.match(/"requestId":"([^"]+)"/)[1],prefix=prompt.match(/新 ID 必须以 (b[0-9]+_) 开头/)[1];
          const catalog=JSON.parse(prompt.split('已有节点目录（摘要可能缩短，以原 ID 为准）：')[1].split('\\n')[0]);
          const parts=JSON.parse(prompt.split('本批资料：')[1].split('\\n')[0]),root=catalog.find(n=>n.parent===null);
          content=JSON.stringify({requestId,currentNodeId:root?.id||prefix+'0',upserts:parts.map((part,i)=>({id:prefix+i,parent:root?.id||(i?prefix+'0':null),lane:i||root?'branch':'main',title:'API 任务 '+(part.messageId||part.partId),summary:'API 模拟摘要',description:'外接 API 模拟结果',status:'待验证',sources:[part.partId]}))});
        }
        if(!body.stream)return new Response(JSON.stringify({choices:[{message:{content}}]}),{headers:{'Content-Type':'application/json'}});
        let at=0;const encoder=new TextEncoder();
        return new Response(new ReadableStream({async pull(controller){
          await new Promise(resolve=>setTimeout(resolve,15));
          if(at<content.length){const text=content.slice(at,at+64);at+=64;controller.enqueue(encoder.encode('data: '+JSON.stringify({choices:[{delta:{content:text}}]})+'\\n\\n'));}
          else{controller.enqueue(encoder.encode('data: '+JSON.stringify({choices:[{delta:{},finish_reason:'stop'}]})+'\\n\\ndata: [DONE]\\n\\n'));controller.close();}
        }}),{headers:{'Content-Type':'text/event-stream'}});
      }})};
      export function ESt(){return manager;}
      export function x3(){throw Error('Must not attach visible side task');}; export function w3(){};
      export function rMt(){throw Error('Must not open side tab');}
      function checkModel(mode){if(mode?.settings.model!=='gpt-5.6-luna'||mode?.settings.reasoning_effort!=='medium')throw Error('Wrong organizer model');}
      export async function Q1(scope,host,options,hooks){
        checkModel(options.collaborationMode);
        if(options.sourceConversationId||options.input.length||!options.sideConversation)throw Error('Blank side contract violated');
        const sideId='fixture-blank-'+(++sideCount);sides.set(sideId,{ephemeral:true,sideConversation:true,turns:[]});await hooks.afterConversationCreated(sideId);return {status:'created',conversationId:sideId,firstTurn:{status:'not-requested'}};
      }

      export async function Mr(options){
        checkModel(options.activeCollaborationMode);checkModel(options.context.collaborationMode);
        if(!sides.has(options.targetConversationId))throw Error('Main thread submission forbidden');
        document.getElementById('status').textContent='整理请求已提交至独立侧边对话';
        const requestId=options.context.prompt.match(/"requestId":"([^"]+)"/)[1];
        const prompt=options.context.prompt,params=new URLSearchParams(location.search);
        const prefix=prompt.match(/新 ID 必须以 (b[0-9]+_) 开头/)[1];
        const catalog=JSON.parse(prompt.split('已有节点目录（摘要可能缩短，以原 ID 为准）：')[1].split('\\n')[0]);
        const parts=JSON.parse(prompt.split('本批资料：')[1].split('\\n')[0]);
        const root=catalog.find(n=>n.parent===null),tree=params.has('tree');
        const upserts=parts.map((part,i)=>({id:prefix+i,parent:root?.id||(i?(tree&&i<4?prefix+(i-1):prefix+'0'):null),lane:!root&&!i?'main':'branch',title:part.text.slice(0,35),summary:'消息 '+part.messageId+' · 片段 '+part.start+'–'+part.end,description:'这是模型接口模拟结果，用于核验分批覆盖、任务树及来源跳转。',status:'待验证',sources:[part.partId]}));
        const result={requestId,currentNodeId:upserts.at(-1).id,upserts};
        const turn={turnId:'fixture-turn-'+(++turnCount),clientUserMessageId:options.clientUserMessageId,status:'inProgress',items:[]};
        sides.get(options.targetConversationId).turns.push(turn);
        const finish=()=>{if(params.has('failonce')&&prefix==='b2_'&&!sessionStorage.getItem('failed-once')){sessionStorage.setItem('failed-once','true');turn.status='failed';}else{turn.status='completed';turn.items=[{type:'agentMessage',phase:'final_answer',text:JSON.stringify(result)}];}};
        if(params.has('slow'))setTimeout(finish,1200);else finish();
        document.getElementById('status').textContent='已向侧边对话提交 '+turnCount+' 批，创建 '+sideCount+' 个侧边对话';
        return turn.turnId;
      }
    `);return;
  }
  if(u.pathname==='/tabs-fixture.js'||u.pathname==='/content-fixture.js'){
    res.setHeader('Content-Type','text/javascript;charset=utf-8');res.end(u.pathname==='/tabs-fixture.js'?'export function i(){}; export const o={};':'export function SideChatTabContent(){}');return;
  }
  if(u.pathname==='/fixture.js'){
    res.setHeader('Content-Type','text/javascript;charset=utf-8');
    res.end(`
    let online=true;
    const rows=[{type:'response_item',timestamp:'2026-09-06',payload:{type:'message',role:'user',id:'m1',content:[{text:'验证连接失败后重新点击能够恢复画布。'}]}}];
    if(new URLSearchParams(location.search).has('tree')){
      rows[0].payload.content=[{text:'需要实现可靠的对话画布。'}];
      for(const [i,text] of ['尝试方案 A：本地规则。','关键词无法识别上下文，A1 失败。','继续改进 A1，结合上下文，尚在试验。','备选方案 B 使用模型归纳，待验证。'].entries())rows.push({type:'response_item',payload:{type:'message',role:'user',id:'m'+(i+2),content:[{text}]}});
    }
    if(new URLSearchParams(location.search).has('long')){
      for(let i=2;i<=260;i++)rows.push({type:'response_item',payload:{type:'message',role:'user',id:'m'+i,content:[{text:'任务 '+i+' 的方案、尝试与结果。'+('完整内容。'.repeat(i===260?10000:200))+(i===260?'长对话末尾验收标记':'')}]}});
    }
    const canvasFixtureReady=(async()=>{
      if(!new URLSearchParams(location.search).has('canvas'))return;
      const specs=[['root',null,'对话决策画布','把目标、尝试与证据组织为任务树','目标已确认'],['a','root','方案 A · 本地规则','尝试关键词和固定规则归类','已停止'],['a1','a','关键词匹配','能够标记主题，无法识别转向原因','验证失败'],['a2','a','加入上下文窗口','解决部分漏判，维护成本仍较高','暂缓'],['b','root','方案 B · 模型整理','按批次识别任务关系与尝试结果','执行中'],['b1','b','分批读取长对话','保留全部片段及对应原文位置','已验证'],['b2','b','增量更新任务树','只归纳新增消息，保留历史尝试','已验证'],['b3','b','外接 API','独立配置地址与模型，后台整理','待验证'],['c','root','可缩放任务画布','全宽树形图与节点详情','执行中'],['c1','c','平移、缩放和定位','通过交互浏览主线与方案分支','当前推进']];
      const nodes=specs.map(([id,parent,title,summary,status],i)=>({id,parent,title,summary,status,description:summary+'。这是一组用于界面验收的演示数据；展开节点可查看其依据。',lane:parent?'branch':'main',sources:['m'+(i%5+1)]}));
      const db=await new Promise((resolve,reject)=>{const request=indexedDB.open('conversation-canvas',1);request.onupgradeneeded=()=>request.result.createObjectStore('records');request.onsuccess=()=>resolve(request.result);request.onerror=()=>reject(request.error);});
      await new Promise((resolve,reject)=>{const tx=db.transaction('records','readwrite');tx.objectStore('records').put({version:3,published:{schemaVersion:2,nodes,currentNodeId:'c1'},job:null},'tree:${id}');tx.oncomplete=resolve;tx.onerror=reject;});db.close();
    })();
    window.__codexSessionDeleteBridge=async (path,payload)=>{
      await canvasFixtureReady;
      if(path==='/diagnostics/log')return {status:'ok'};
      if(path!=='/session/export')throw Error('Unexpected bridge call');
      if(new URLSearchParams(location.search).has('oversize'))return {status:'error',message:'会话文件超过分享大小限制'};
      if(!online)return {status:'failed',message:'模拟会话接口暂不可用'};
      return {status:'ok',kind:'codex-rollout',session_id:payload.session_id,content:rows.map(r=>JSON.stringify(r)+'\\n').join('')};
    };
    document.getElementById('theme-toggle').onclick=()=>document.documentElement.classList.toggle('dark');
    document.getElementById('service-toggle').onclick=()=>{online=!online;document.getElementById('status').textContent=online?'会话接口已恢复':'会话接口已断开';};
    document.getElementById('add-message').onclick=()=>{rows.push({type:'response_item',timestamp:'2026-09-06',payload:{type:'message',role:'user',id:'m'+(rows.length+1),content:[{text:'新的方案尝试 '+rows.length}]}});};
    `);return;
  }
  if(u.pathname==='/host'){
    res.setHeader('Content-Type','text/html;charset=utf-8');
    res.end(`<!doctype html><meta charset="utf-8"><style>
    :root{--color-token-main-surface-primary:#fff;--color-token-text-primary:#242424;--color-token-text-secondary:#737373;--color-token-border:#e6e6e6;--color-token-list-hover-background:#80808018;font:13px "Segoe UI",sans-serif;background:var(--color-token-main-surface-primary);color:var(--color-token-text-primary)}
    :root.dark{--color-token-main-surface-primary:#202020;--color-token-text-primary:#ededed;--color-token-text-secondary:#aaa;--color-token-border:#3a3a3a}
    body{margin:0}header{height:52px;border-bottom:1px solid var(--color-token-border);display:flex;align-items:center;padding:0 18px}header>div{display:flex;align-items:center;width:100%;gap:8px}.ms-auto{margin-left:auto;display:flex;gap:6px;align-items:center}button{font:inherit;background:none;color:inherit;border:1px solid var(--color-token-border);padding:5px 9px;border-radius:6px;cursor:pointer}main{padding:30px;max-width:550px;line-height:1.8}article{padding:20px 0}
    </style><header><div data-testid="app-shell-header-context-menu-surface"><span>当前任务</span><div class="ms-auto"><button aria-label="Share">分享</button></div></div></header><main><h1>原生画布 CSP 回归测试</h1><p>本页同时启用 frame-src 'none' 和 connect-src 'none'。</p><button id="theme-toggle">切换深浅色</button> <button id="service-toggle">切换会话接口状态</button> <button id="add-message">追加新消息</button><p id="status">会话接口已恢复</p><article id="m1">验证连接失败后重新点击能够恢复画布。</article></main><script src="/fixture.js"></script><script src="/test-user.js"></script>`);return;
  }
  if(u.pathname==='/test-user.js'){
    res.setHeader('Content-Type','text/javascript;charset=utf-8');
    res.end(fs.readFileSync(new URL('canvas.user.js',root),'utf8').replace('!/^app:\\/\\/-/.test(location.href)','location.hostname!=="127.0.0.1"').replaceAll('app://-/assets/app-initial-f87238153a19.js','/native-fixture.js').replace('app://-/assets/thread-pin-shortcut-bridge-f85d28ddca38.js','/tabs-fixture.js').replace('app://-/assets/side-chat-tab-content-7dc20bc79612.js','/content-fixture.js'));return;
  }
  res.writeHead(404);res.end();
}).listen(port,'127.0.0.1',()=>console.log(`http://127.0.0.1:${port}/host?thread=${id}`));
