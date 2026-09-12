// ==UserScript==
// @name         Conversation canvas
// @description  可缩放的任务树画布与原文定位
// @version      0.3.1
// ==/UserScript==
(function installCanvas() {
  if(!document.body){window.addEventListener('DOMContentLoaded',installCanvas,{once:true});return;}
  if(window.top!==window || !/^app:\/\/-/.test(location.href))return;
  window.__conversationCanvasCleanup?.();
  const previous=document.getElementById('conversation-canvas-host');
  previous?.shadowRoot?.querySelector('aside')?.classList.remove('open');
  previous?.remove();
  /* canvas-model */
  /* canvas-api */
  /* canvas-interaction */
  /* canvas-annotations */
  function diagnostic(stage,detail={}){
    try{window.__codexSessionDeleteBridge?.('/diagnostics/log',{event:'conversation_canvas',detail:{version:'0.3.1',stage,...detail}})?.catch(()=>{});}catch{}
  }
  diagnostic('native_installed',{pageOrigin:location.origin});
  const host=document.createElement('div');host.id='conversation-canvas-host';
  const shadow=host.attachShadow({mode:'open'});
  shadow.innerHTML=`<style>
    :host{--panel-bg:var(--color-token-main-surface-primary,#fff);--panel-fg:var(--color-token-text-primary,#242424);--panel-border:var(--color-token-border,#e5e5e5);font-family:inherit;color:var(--panel-fg)}
    aside{position:fixed;right:0;top:var(--canvas-top,48px);bottom:0;width:min(420px,calc(100vw - 24px));background:var(--panel-bg);border-left:1px solid var(--panel-border);display:none;z-index:40;box-sizing:border-box}
    aside.open{display:flex;flex-direction:column}
    .panel-header{display:flex;align-items:center;gap:8px;padding:10px 12px;border-bottom:1px solid var(--panel-border);min-height:48px;box-sizing:border-box}
    .heading{flex:1;min-width:0}.heading strong{font-size:13px;font-weight:600}.heading p{font-size:11px;line-height:1.4;margin:3px 0 0;color:var(--color-token-text-secondary,#777);overflow-wrap:anywhere}
    .panel-header button{font:inherit;color:inherit;background:transparent;border:0;border-radius:6px;width:28px;height:28px;cursor:pointer;display:grid;place-items:center}
    button:hover{background:var(--color-token-list-hover-background,#0000000a)}button:focus-visible{outline:2px solid currentColor;outline-offset:2px}
    [hidden]{display:none!important}
    .canvas-body{position:relative;display:flex;flex-direction:column;flex:1;min-height:0}
    .canvas-toolbar{display:flex;gap:16px;padding:0 16px;border-bottom:1px solid var(--panel-border)}
    .canvas-toolbar button{border:0;border-bottom:2px solid transparent;background:none;color:var(--color-token-text-secondary,#777);font:inherit;font-size:12px;line-height:1.5;padding:10px 0;cursor:pointer}
    .canvas-toolbar button:disabled{opacity:.5;cursor:wait}#organize{margin-left:auto}#organization-status{padding:8px 16px;margin:0;font-size:11px;line-height:1.5;border-bottom:1px solid var(--panel-border)}.canvas-toolbar button.active{border-bottom-color:currentColor;color:var(--panel-fg)}
    .canvas-scroll{overflow:auto;flex:1;min-height:0;padding:16px;font-size:12px;line-height:1.7;scroll-behavior:smooth}
    h2{font-size:15px;margin:0 0 6px}#subtitle,.note,small{color:var(--color-token-text-secondary,#777);font-size:11px}.note{margin-top:16px}#graph{margin-top:12px}
    .node{font:inherit;display:block;min-width:0;flex:1;padding:10px 12px;text-align:left;background:var(--panel-bg);color:var(--panel-fg);border:1px solid var(--panel-border);border-radius:8px;cursor:pointer;overflow-wrap:anywhere}
    .node strong,.node small{display:block}.node p{font-size:11px;margin:5px 0 0;color:var(--color-token-text-secondary,#777)}.node strong{font-size:13px;margin-top:4px}.node.selected{outline:1px solid var(--color-token-text-secondary,#777)}
    .node.current{border-color:var(--color-token-text-success,#25805c)}.node.current small{color:var(--color-token-text-success,#25805c)}
    .task-tree,.tree-children{list-style:none;padding:0;margin:0}.tree-children{margin:8px 0 0 11px;padding-left:14px;border-left:1px solid var(--panel-border)}.task-branch{margin:0 0 10px;position:relative}.tree-children>.task-branch:before{content:"";position:absolute;top:22px;left:-14px;width:14px;border-top:1px solid var(--panel-border)}
    .tree-row{display:flex;align-items:flex-start;gap:3px}.tree-toggle,.tree-leaf{flex:0 0 21px;box-sizing:border-box;width:21px;margin-top:10px;text-align:center}.tree-toggle{font:inherit;border:0;background:none;color:inherit;cursor:pointer;padding:0;height:24px}.active-path>.tree-children{border-left-color:var(--color-token-text-success,#25805c)}
    .tree-actions{display:flex;gap:10px;margin-top:12px}.tree-actions button{font:inherit;font-size:11px;color:inherit;border:1px solid var(--panel-border);border-radius:5px;background:none;padding:3px 7px;cursor:pointer}.tree-actions button:disabled{opacity:.45;cursor:default}
    #source-view>.tree-actions,#tree-pages{position:sticky;top:0;z-index:2;background:var(--panel-bg);padding:8px 0}
    .pending-messages{margin-top:20px;border-top:1px solid var(--panel-border);padding-top:10px}.pending-messages>summary{cursor:pointer;color:var(--color-token-text-secondary,#777)}.pending-list{display:grid;gap:8px}#detail-path{font-size:11px;color:var(--color-token-text-secondary,#777)}
    #details{position:absolute;bottom:8px;left:8px;right:8px;max-height:65%;overflow:auto;background:var(--panel-bg);border:1px solid var(--panel-border);border-radius:10px;padding:16px;box-shadow:0 -6px 24px #0002;font-size:12px;line-height:1.8}
    .detail-head{display:flex;justify-content:space-between;align-items:center}.detail-head button{border:0;background:none;color:inherit;font-size:22px;cursor:pointer}#detail-description{white-space:pre-wrap;overflow-wrap:anywhere}
    #source-actions{display:flex;gap:6px;flex-wrap:wrap}#source-actions button{font:inherit;background:transparent;color:inherit;border:1px solid var(--panel-border);border-radius:6px;padding:5px 9px;cursor:pointer}
    #jump-status{font-size:11px;color:var(--color-token-text-secondary,#777)}.message{border-bottom:1px solid var(--panel-border);padding:14px 0;scroll-margin-top:12px}.message pre{font:inherit;white-space:pre-wrap;overflow-wrap:anywhere}.message.highlight{outline:4px solid var(--panel-border);background:var(--color-token-list-hover-background,#80808018)}

  /* canvas-style */
  </style><aside role="complementary" aria-label="对话脉络" data-version="0.3.1"><div class="panel-header"><div class="heading"><strong>对话脉络</strong><p id="connection" role="status">当前对话的主线与分支</p></div><button id="retry" aria-label="重新连接" title="重新连接">↻</button><button id="close" aria-label="关闭画布" title="关闭">×</button></div><div class="canvas-body"><div class="canvas-toolbar"><button id="map-tab" class="active">任务树</button><button id="source-tab">对话原文</button><button id="pause-organize" hidden>暂停整理</button><button id="organize" title="GPT-5.6 Luna · 中 · 后台分批整理">整理脉络</button></div><p id="organization-status" role="status" hidden></p><div class="canvas-scroll"><section id="map-view"><div class="map-topbar"><div class="map-heading"><h2 id="title">当前对话</h2><div id="subtitle"></div></div><div class="tree-actions"><button id="expand-tree">展开全部</button><button id="collapse-tree">收起分支</button><button id="locate-current">当前推进</button><button id="fit-canvas">适应视图</button></div></div><div id="graph" role="region" aria-label="任务树画布，滚轮缩放，拖拽平移" tabindex="0"></div><div id="pending-tray"></div><div class="canvas-bottom"><span class="canvas-help">拖动画布平移 · 滚轮缩放 · 点击节点查看依据</span><div class="zoom-tools"><button id="zoom-out" aria-label="缩小画布">−</button><button id="canvas-zoom" title="恢复 100% 缩放">100%</button><button id="zoom-in" aria-label="放大画布">+</button></div></div></section><section id="source-view" hidden><div class="tree-actions"><button id="source-prev">上一页</button><span id="source-page-info"></span><button id="source-next">下一页</button><button id="source-list">消息列表</button></div><div id="transcript"></div></section></div><section id="details" aria-label="节点详情" hidden><div class="detail-head"><small id="detail-status"></small><button id="close-detail" aria-label="关闭节点详情">×</button></div><h2 id="detail-title"></h2><p id="detail-path"></p><p id="detail-description"></p><div id="source-actions"></div><p id="jump-status" role="status"></p></section></div></aside>`;
  document.body.append(host);
  const pane=shadow.querySelector('aside');
  pane.dataset.version='0.3.1';
  const settingsButton=document.createElement('button');settingsButton.id='organizer-settings';settingsButton.textContent='⚙';settingsButton.title='整理设置';settingsButton.setAttribute('aria-label','整理设置');
  shadow.getElementById('retry').before(settingsButton);
  const settingsForm=document.createElement('form');settingsForm.id='api-settings';settingsForm.hidden=true;settingsForm.setAttribute('aria-label','整理设置');
  settingsForm.innerHTML=`<h2>整理设置</h2><fieldset id="api-fields"><label>整理通道<select id="api-channel"><option value="native">Codex 后台 · 5.6 Luna · 中</option><option value="external">外接 API · OpenAI 兼容</option></select></label><div id="api-external" hidden><label>API 地址<input id="api-url" type="url" placeholder="https://api.example.com/v1" autocomplete="off"></label><label>模型名称<input id="api-model" placeholder="服务商提供的模型 ID" autocomplete="off"></label><label>整理速度<select id="api-speed"><option value="fast">快速整理（DeepSeek V4 关闭思考）</option><option value="provider">服务商默认推理</option></select></label><p class="note">快速模式保留来源校验；其他服务商的推理参数保持默认。临时连接错误最多自动重试 2 次。</p><label>API Key<input id="api-key" type="password" autocomplete="off" spellcheck="false"></label><label class="remember-key"><input id="api-remember" type="checkbox">记住密钥（在本机明文保存）</label><p class="note">默认仅本次加载有效。整理时会将待整理对话及相关节点摘要发送至上面的 API 地址；连接测试仅发送一条测试指令。</p><button type="button" id="api-test">测试连接</button></div><p id="api-native-note" class="note">静默分批整理，固定使用 GPT-5.6 Luna、中等推理。</p><div class="tree-actions"><button type="submit">保存设置</button><button type="button" id="api-cancel">返回任务树</button></div></fieldset><p id="api-status" role="status"></p>`;
  shadow.querySelector('.canvas-body').append(settingsForm);
  const settingsStyle=document.createElement('style');settingsStyle.textContent='#api-settings{position:absolute;inset:0;z-index:4;overflow:auto;padding:16px;background:var(--panel-bg);font-size:12px;line-height:1.6}#api-settings fieldset{border:0;padding:0;margin:0;min-width:0}#api-settings label{display:block;margin:12px 0}#api-settings input:not([type=checkbox]),#api-settings select{box-sizing:border-box;width:100%;padding:7px 8px;margin-top:4px;background:var(--panel-bg);color:inherit;border:1px solid var(--panel-border);border-radius:6px;font:inherit}#api-settings .remember-key{display:flex;align-items:center;gap:6px;font-size:11px}#api-settings button{font:inherit;border:1px solid var(--panel-border);border-radius:5px;background:none;color:inherit;padding:5px 8px;cursor:pointer}#api-settings button:disabled{opacity:.5}#api-status{overflow-wrap:anywhere}';shadow.append(settingsStyle);
  const loadingText=shadow.getElementById('connection'),retry=shadow.getElementById('retry');
  // The entry lives in the real toolbar. The panel is separate to avoid its paint containment.
  const toggle=document.createElement('button');toggle.id='conversation-canvas-toggle';toggle.type='button';
  toggle.className='user-select-none no-drag cursor-interaction flex shrink-0 items-center gap-1 whitespace-nowrap rounded-lg text-token-button-tertiary-foreground h-token-button-composer px-2 py-0 text-base leading-[18px]';
  toggle.setAttribute('aria-label','对话脉络');toggle.setAttribute('aria-expanded','false');toggle.title='打开当前对话的任务树画布';
  toggle.innerHTML='<svg aria-hidden="true" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round"><circle cx="6" cy="5" r="2"/><circle cx="6" cy="19" r="2"/><circle cx="18" cy="12" r="2"/><path d="M6 7v10M6 9c0 3 5 3 10 3"/></svg><span>对话脉络</span>';
  const entryStyle=document.createElement('style');
  entryStyle.textContent='#conversation-canvas-toggle{position:static;pointer-events:auto;-webkit-app-region:no-drag;display:inline-flex;align-items:center;gap:5px;flex-shrink:0;font:inherit;font-size:12px;height:28px;padding:0 8px;border:1px solid transparent;border-radius:6px;color:var(--color-token-text-secondary,inherit);background:transparent;cursor:pointer}#conversation-canvas-toggle:hover,#conversation-canvas-toggle[aria-expanded="true"]{color:var(--color-token-text-primary,inherit);background:var(--color-token-list-hover-background,#80808018)}#conversation-canvas-toggle:focus-visible{outline:2px solid currentColor;outline-offset:2px}#conversation-canvas-toggle[hidden]{display:none}';
  document.head.append(entryStyle);
  let toolbar=null,ownedGroup=null,mountFrame=0;
  const boundsObserver=new ResizeObserver(updatePanelBounds);
  function visible(e){const r=e.getBoundingClientRect();return r.width>0&&r.height>0&&!e.closest('[aria-hidden="true"]');}
  function mountEntry(){
    mountFrame=0;
    const surface=[...document.querySelectorAll('[data-testid="app-shell-header-context-menu-surface"]')].find(visible);
    const header=surface?.closest('header')||[...document.querySelectorAll('header')].find(visible);
    const container=surface||header;
    if(!container){toggle.remove();return;}
    let group=[...container.querySelectorAll('.ms-auto')].find(visible);
    if(!group){
      if(!ownedGroup||ownedGroup.parentElement!==container){ownedGroup?.remove();ownedGroup=document.createElement('div');ownedGroup.className='ms-auto flex shrink-0 items-center gap-1.5';ownedGroup.setAttribute('data-app-shell-header-obstacle','true');ownedGroup.style.cssText='margin-left:auto;display:flex;flex-shrink:0;align-items:center;pointer-events:auto';container.append(ownedGroup);}
      group=ownedGroup;
    }
    if(toggle.parentElement!==group)group.append(toggle);
    const nextToolbar=header||surface;
    if(toolbar!==nextToolbar){boundsObserver.disconnect();if(nextToolbar)boundsObserver.observe(nextToolbar);toolbar=nextToolbar;}
    toggle.hidden=!currentId();
    updatePanelBounds();
  }
  function scheduleMount(){if(!mountFrame)mountFrame=requestAnimationFrame(mountEntry);}
  function updatePanelBounds(){
    const bounds=toolbar?.getBoundingClientRect();
    const bottom=bounds?.bottom;
    const left=Math.max(0,Math.round(bounds?.left||0));
    if(host.style.getPropertyValue('--canvas-left')!==`${left}px`)host.style.setProperty('--canvas-left',`${left}px`);
    const value=Number.isFinite(bottom)&&bottom>0&&bottom<180?bottom:48;
    const next=`${Math.round(value)}px`;if(host.style.getPropertyValue('--canvas-top')!==next)host.style.setProperty('--canvas-top',next);
  }
  const $=id=>shadow.getElementById(id);
  const uuid=/[a-f0-9]{8}-[a-f0-9]{4}-[a-f0-9]{4}-[a-f0-9]{4}-[a-f0-9]{12}/i;
  let active='',data=null,selected=null,revision='',pending=null,requestSequence=0,disposed=false,lastRead=0;
  let organizing=null,historyAbort=null;
  const nativeReadTasks=new Set(),historyCache=persistentHistoryCache();
  let organizationAbort=new AbortController();
  let organizerConfig={channel:'native'},testingApi=false,activeApiTester=null;
  const externalOrganizer=createApiOrganizer({request:nativeApiRequest});
  const settingsReady=canvasStore('organizer-settings').then(value=>{if(value)organizerConfig=value;updateOrganizerTitle();}).catch(()=>{$('api-status').textContent='设置读取失败，请重新保存配置。';});
  function updateOrganizerTitle(){$('organize').title=organizerConfig.channel==='external'?`外接 API · ${organizerConfig.model||'待配置'} · 后台分批整理`:'GPT-5.6 Luna · 中 · 后台分批整理';}
  function showApiFields(){const external=$('api-channel').value==='external';$('api-external').hidden=!external;$('api-native-note').hidden=external;for(const field of $('api-external').querySelectorAll('input,select,button'))field.disabled=!external;}
  function readApiForm(){return apiConfig({channel:$('api-channel').value,baseUrl:$('api-url').value,model:$('api-model').value,key:$('api-key').value,remember:$('api-remember').checked,speed:$('api-speed').value});}
  settingsButton.onclick=async()=>{await settingsReady;if(disposed||organizing)return;for(const [name,value] of [['channel',organizerConfig.channel],['url',organizerConfig.baseUrl],['model',organizerConfig.model],['key',organizerConfig.key]])$(`api-${name}`).value=value||'';$('api-remember').checked=!!organizerConfig.remember;$('api-speed').value=organizerConfig.speed||'fast';showApiFields();settingsForm.hidden=false;};
  $('api-channel').onchange=showApiFields;
  $('api-cancel').onclick=()=>{settingsForm.hidden=true;$('api-key').value='';};
  settingsForm.onsubmit=async event=>{event.preventDefault();if(organizing||testingApi)return;try{const next=readApiForm();await canvasStore('organizer-settings',storedApiConfig(next));externalOrganizer.dispose();organizerConfig=next;updateOrganizerTitle();$('api-status').textContent='设置已保存。返回任务树后点击整理脉络。';$('api-key').value='';settingsForm.hidden=true;organizationStatus(`已切换至 ${next.channel==='external'?`外接 API · ${next.model}`:'Codex 后台 · 5.6 Luna · 中'}，点击整理后生效。`);}catch(error){$('api-status').textContent=error.message;}};
  $('api-test').onclick=async()=>{
    if(organizing||testingApi)return;testingApi=true;$('api-fields').disabled=true;$('organize').disabled=true;
    const tester=createApiOrganizer({request:nativeApiRequest});activeApiTester=tester;
    try{const config=readApiForm();$('api-status').textContent='正在测试连接…';await tester.run(config,'Reply with OK only.',crypto.randomUUID(),{},async()=>{},undefined);if(!disposed)$('api-status').textContent='连接成功，模型已返回文本。点击保存设置后生效。';}
    catch(error){if(!disposed)$('api-status').textContent=error.message;}
    finally{tester.dispose();activeApiTester=null;testingApi=false;if(!disposed){$('api-fields').disabled=false;$('organize').disabled=false;}}
  };
  const recordCache=new Map(),annotationCache=new Map();
  let pendingOffset=0,pendingOpen=false,sourceState={page:0};
  let collapsedNodes=new Set();
  const collapsedByThread=new Map();
  const taskCanvas=createTaskCanvas($('graph'),{
    onNode:id=>detail(id),
    onCollapse:id=>{collapsedNodes.has(id)?collapsedNodes.delete(id):collapsedNodes.add(id);renderTree();},
    onCamera:camera=>{$('canvas-zoom').textContent=`${Math.round(camera.scale*1000)/10}%`;}
  });
  const storedAnnotations=id=>annotationCache.get(id)||annotationIndex[id]||{nodes:[]};
  async function hydrateAnnotations(id){
    const record=await canvasStore(`tree:${id}`);recordCache.set(id,record);
    if(active===id&&!organizing){
      $('organize').textContent=record?.job?'继续整理':record?.published?.done?'增量整理':'整理脉络';
      if(!data&&record?.job)organizationStatus(`已恢复整理进度 · 已保存 ${record.job.state.batchCount} 批，点击「继续整理」接续${record.job.kind==='rebuild'?'；原任务树保留至重新整理完成':''}`);
    }
    if(record?.published){annotationCache.set(id,record.published);return;}
    try{const legacy=JSON.parse(localStorage.getItem(`conversation-canvas:organized:${id}`));if(legacy?.threadId===id&&Array.isArray(legacy.nodes))annotationCache.set(id,legacy);}catch{}
  }
  function organizationStatus(text){$('organization-status').hidden=!text;$('organization-status').textContent=text;}
  async function organize(){
    if(organizing||testingApi)return;
    const id=currentId();if(!id)return;
    organizing=id;organizationAbort=new AbortController();const signal=organizationAbort.signal;
    $('organize').disabled=true;settingsButton.disabled=true;$('pause-organize').hidden=false;
    try{
      await settingsReady;const config=apiConfig(organizerConfig);
      organizationStatus('正在读取待整理资料…');
      await sync(true);
      while(pending===id){signal.throwIfAborted();await new Promise(resolve=>setTimeout(resolve,250));}
      if(!data?.messages.length||data.threadId!==id||currentId()!==id)throw Error('当前对话尚未读取完成，请稍后重试');
      await hydrateAnnotations(id);
      const snapshot=data,old=storedAnnotations(id);
      const record=recordCache.get(id)||{published:old.nodes.length?old:null};
      const applyGraph=result=>{annotationCache.set(id,result);if(!disposed&&active===id){data={...data,...buildGraph(data.messages,result)};selected=null;$('details').hidden=true;render();status(`已同步${nativeReadTasks.has(id)?'（原生分页）':''} · ${result.nodes.length} 个任务节点`);}};
      const showProgress=text=>{if(active!==id)return;const job=recordCache.get(id)?.job;const repair=job?.pending?.repair;organizationStatus(`${job?.kind==='rebuild'?'重整中，当前展示上次结果 · ':''}${repair?`补正 ${repair.attempt}/2 · `:''}${text}`);};
      const saved=await organizeLong({messages:snapshot.messages,record,signal,compact:config.channel==='external',
        save:async document=>{await canvasStore(`tree:${id}`,document);recordCache.set(id,document);},
        progress:showProgress,onGraph:applyGraph,
        run:(prompt,requestId,session,onSession)=>config.channel==='external'
          ?externalOrganizer.run(config,prompt,requestId,session,onSession,signal,text=>showProgress(`第 ${(recordCache.get(id)?.job?.state?.batchCount||0)+1} 批 · ${text}`))
          :nativeSideChat(id,prompt,text=>showProgress(`第 ${(recordCache.get(id)?.job?.state?.batchCount||0)+1} 批 · ${text}`),signal,{session,onSession,requestId})});
      if(!disposed&&active===id)organizationStatus(`整理完成 · ${saved.published.nodes.length} 个任务节点，进度已保存；新增消息可增量整理`);
    }catch(error){if(!disposed&&active===id)organizationStatus(error.name==='AbortError'?'已暂停，已完成批次保留。后台模型可能仍在运行，点击继续可接收结果。':`整理暂未完成：${error.message}。已完成批次保留，点击继续重试。`);}
    finally{organizing=null;if(!disposed){$('organize').disabled=false;settingsButton.disabled=false;$('organize').textContent=recordCache.get(active)?.job?'继续整理':'增量整理';$('pause-organize').hidden=true;}}
  }
  function currentId(){
    const route=(location.href.match(uuid)||[])[0];if(route)return route;
    const rows=[...document.querySelectorAll('[data-app-action-sidebar-thread-id]')].filter(row=>['page','true'].includes(row.getAttribute('aria-current'))||row.querySelector('[aria-current="page"],[aria-current="true"]'));
    const ids=[...new Set(rows.map(row=>(`${row.getAttribute('data-app-action-sidebar-thread-id')} ${row.getAttribute('href')} ${row.querySelector('a')?.getAttribute('href')}`.match(uuid)||[])[0]).filter(Boolean))];
    return ids.length===1?ids[0]:'';
  }
  const esc=s=>String(s??'').replace(/[&<>"']/g,c=>({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[c]));
  function status(text){loadingText.textContent=text;}
  function renderTree(){
    if(!data)return;
    const layout=taskCanvas.update(data.nodes,data.currentNodeId,collapsedNodes,selected);
    pendingOffset=Math.min(pendingOffset,Math.max(0,Math.ceil(layout.pending.length/20)-1)*20);
    const pendingMarkup=treeMarkup(layout.pending,null,new Set(),selected,{pendingOffset,pendingOpen});
    $('pending-tray').innerHTML=layout.pending.length?pendingMarkup.slice(pendingMarkup.indexOf('<details')):'';
    $('locate-current').disabled=!data.currentNodeId;
    $('expand-tree').disabled=!layout.cards.length;$('collapse-tree').disabled=!layout.cards.length;
  }
  function renderTranscript(){
    if(!data)return;
    const result=transcriptPage(data.messages,sourceState);
    $('transcript').innerHTML=result.html;$('source-page-info').textContent=result.info;
    $('source-prev').disabled=result.page===0;$('source-next').disabled=result.page+1>=result.total;$('source-list').hidden=!result.focused;
    if(result.focused)sourceState.part=result.page;else sourceState.page=result.page;
  }
  function render(){
    $('title').textContent=data.title;
    const pendingCount=data.nodes.filter(n=>n.summaryKind==='原文摘录').length;
    $('subtitle').textContent=`${data.messages.length} 条消息 · ${data.nodes.length-pendingCount} 个任务节点 · ${pendingCount} 条待整理`;
    renderTree();if(!$('source-view').hidden)renderTranscript();
  }
  function tab(which){$('map-view').hidden=which!=='map';$('source-view').hidden=which!=='source';$('map-tab').classList.toggle('active',which==='map');$('source-tab').classList.toggle('active',which==='source');shadow.querySelector('.canvas-scroll').classList.toggle('showing-source',which==='source');if(which==='source')renderTranscript();else taskCanvas.refresh();}
  function showSource(id,start=0){$('details').hidden=true;const index=data.messages.findIndex(m=>m.id===id);sourceState={page:Math.max(0,Math.floor(index/20)),focusId:id,part:Math.floor(start/12000)};tab('source');$(`source-${id}`)?.scrollIntoView({block:'start'});}
  function detail(id,sourceOffset=0){
    const n=data?.nodes.find(n=>n.id===id);if(!n)return;selected=id;taskCanvas.select(id);
    shadow.querySelectorAll('[data-node]').forEach(e=>e.classList.toggle('selected',e.dataset.node===id));
    const path=[],seen=new Set(),byId=new Map(data.nodes.map(node=>[node.id,node]));let ancestor=n;
    while(ancestor&&!seen.has(ancestor.id)){seen.add(ancestor.id);path.push(ancestor.title);ancestor=byId.get(ancestor.parent);}path.reverse();
    $('detail-path').textContent=n.summaryKind==='原文摘录'?'待识别任务归属':path.join(' › ');
    $('detail-title').textContent=n.title;$('detail-status').textContent=n.status;$('detail-description').textContent=n.description.slice(0,10000)+(n.description.length>10000?'\n（更多内容可分段查看对应原文）':'')+(n.revisions?.length?`\n\n已保留 ${n.revisions.length} 次历史摘要更新及其原文来源。`:'');$('details').hidden=false;$('jump-status').textContent='';
    $('source-actions').replaceChildren();
    for(const [index,id] of n.sources.slice(sourceOffset,sourceOffset+10).entries()){
      const i=index+sourceOffset;
      const m=data.messages.find(m=>m.id===id);if(!m)continue;
      const source=document.createElement('button');source.textContent=n.sources.length>1?`查看原文 ${i+1}`:'查看对应原文';source.onclick=()=>showSource(id,n.evidence?.find(e=>e.messageId===id)?.start||0);$('source-actions').append(source);
      const jump=document.createElement('button');jump.textContent='定位 Codex 对话';const thread=data.threadId;jump.onclick=()=>jumpToMessage(thread,m);$('source-actions').append(jump);
    }
    if(n.sources.length>10){for(const [label,offset] of [['前 10 个来源',Math.max(0,sourceOffset-10)],['后 10 个来源',sourceOffset+10]]){const button=document.createElement('button');button.textContent=label;button.disabled=offset>=n.sources.length||offset===sourceOffset;button.onclick=()=>detail(n.id,offset);$('source-actions').append(button);}}
    if(n.revisions?.length){
      let version=n.revisions.length;const latest=$('detail-description').textContent;
      const older=document.createElement('button'),newer=document.createElement('button');older.textContent='上一次摘要';newer.textContent='下一次摘要';newer.disabled=true;
      const showVersion=()=>{const previous=n.revisions[version];$('detail-description').textContent=previous?`历史摘要 ${version+1}/${n.revisions.length} · ${previous.status}\n${previous.summary}\n\n${previous.description}`:latest;older.disabled=version===0;newer.disabled=version===n.revisions.length;};
      older.onclick=()=>{version--;showVersion();};newer.onclick=()=>{version++;showVersion();};$('source-actions').append(older,newer);
    }
  }
  const normalize=s=>String(s||'').replace(/\s+/g,'').replace(/[*#`]/g,'');
  async function jumpToMessage(threadId,message){
    if(currentId()!==threadId){$('jump-status').textContent='当前任务已切换，请重新打开节点。';return;}
    let target=nativeMessageTarget(document,message.id),navigationState=message.turnId?'unavailable':'missing-turn';
    if(!target&&message.turnId){
      $('jump-status').textContent='正在展开原聊天的对应轮次…';
      try{
        const {navigation}=await nativeContext(threadId);
        if(disposed||currentId()!==threadId)return;
        if(navigation){await navigation.scrollToTurn(message.turnId);navigationState='revealed';}
        if(disposed||currentId()!==threadId)return;
        target=nativeMessageTarget(document,message.id);
      }catch(error){navigationState='failed';diagnostic('native_jump_error',{message:error.message});}
    }
    if(!target){
      const needle=normalize(message.text).slice(0,100);
      if(needle.length>=15){
        const candidates=[...document.querySelectorAll('p,pre,[data-message-id],article')].filter(e=>!host.contains(e)&&normalize(e.textContent).includes(needle));
        const leaves=candidates.filter(e=>!candidates.some(other=>other!==e&&e.contains(other)));
        if(leaves.length===1)target=leaves[0];
      }
    }
    if(!target){$('jump-status').textContent=({revealed:'已展开所属轮次，但该消息仍被折叠或未显示。可先查看对应原文。',unavailable:'当前页面的历史定位尚未就绪。可先查看对应原文。','missing-turn':'该来源缺少轮次信息。可先查看对应原文。',failed:'原聊天轮次展开失败。可先查看对应原文。'})[navigationState];return;}
    closePanel();
    target.scrollIntoView({behavior:'smooth',block:'center'});
    target.animate([{outline:'2px solid #888',backgroundColor:'#8882'},{outline:'2px solid transparent',backgroundColor:'transparent'}],{duration:2000});
    $('jump-status').textContent='正在定位原聊天消息…';
    setTimeout(()=>{if(disposed||currentId()!==threadId)return;const r=target.getBoundingClientRect();$('jump-status').textContent=target.isConnected&&r.height>0&&r.bottom>0&&r.top<innerHeight?'已定位并高亮原聊天消息。':'消息已找到，滚动尚未完成。';},700);
  }
  async function readCanvasSource(id,signal,progress){
    if(!nativeReadTasks.has(id)){
      let result;
      try{
        if(typeof window.__codexSessionDeleteBridge!=='function')throw Error('Codex++ 会话接口尚未连接，请稍后点击刷新');
        let timer;
        try{result=await Promise.race([window.__codexSessionDeleteBridge('/session/export',{session_id:id}),new Promise((_,reject)=>{timer=setTimeout(()=>reject(Error('会话读取超时，请重试')),15000);})]);}
        finally{clearTimeout(timer);}
      }catch(error){if(!isExportSizeError(error.message))throw error;result={message:error.message};}
      if(result?.status==='ok')return result;
      if(!isExportSizeError(result?.message))throw Error(result?.message||'Codex++ 会话读取失败');
      nativeReadTasks.add(id);
    }
    signal.throwIfAborted();progress('会话较大，正在改用原生分页读取…');
    const {manager}=await nativeContext(id);
    const messages=await readNativeHistory((method,params)=>manager.sendRequest(method,params,{priority:'background',timeoutMs:15000}),id,historyCache,progress,signal);
    const signature=[];for(const message of messages){signal.throwIfAborted();signature.push(message.id+':'+await messageDigest(message));}
    return {status:'ok',kind:'canvas-messages',session_id:id,messages,content:signature.join('|')};
  }
  async function sync(force=false){
    const id=currentId();
    if(!id){historyAbort?.abort();requestSequence++;active='';pending=null;data=null;revision='';taskCanvas.context('');$('pending-tray').replaceChildren();$('transcript').replaceChildren();$('details').hidden=true;status('请打开具体任务。');return;}
    if(id!==active){
      historyAbort?.abort();taskCanvas.context(id);pendingOffset=0;pendingOpen=false;sourceState={page:0};collapsedByThread.set(active,collapsedNodes);collapsedNodes=collapsedByThread.get(id)||new Set();active=id;organizationStatus(organizing===id?'GPT-5.6 Luna · 中 · 正在后台整理…':'');requestSequence++;pending=null;data=null;revision='';selected=null;lastRead=0;
      $('pending-tray').replaceChildren();$('transcript').replaceChildren();$('details').hidden=true;$('subtitle').textContent='';$('title').textContent='当前对话';tab('map');
    }
    if(pending===id||(!force&&Date.now()-lastRead<5000))return;
    const seq=++requestSequence;pending=id;lastRead=Date.now();if(!data)status('正在读取当前对话…');
    const abort=new AbortController();historyAbort=abort;
    try{
      await hydrateAnnotations(id);
      const result=await readCanvasSource(id,abort.signal,text=>{if(!disposed&&seq===requestSequence)status(text);});
      if(disposed||seq!==requestSequence||id!==currentId())return;
      if(result?.status!=='ok')throw new Error(result?.message||'Codex++ 会话读取失败');
      if(!data||result.content!==revision){
        data=result.kind==='canvas-messages'?{threadId:id,title:storedAnnotations(id).title||'当前对话',messages:result.messages,...buildGraph(result.messages,storedAnnotations(id))}:graphFromExport(result,id,storedAnnotations(id));revision=result.content;render();
        diagnostic('native_data',{nodeCount:data.nodes.length,messageCount:data.messages.length});
      }
      status(`已同步${nativeReadTasks.has(id)?'（原生分页）':''} · ${data.nodes.filter(n=>n.summaryKind!=='原文摘录').length} 个任务节点`);
    }catch(e){
      if(!disposed&&seq===requestSequence){status(`读取失败：${e.message}`);diagnostic('native_error',{message:e.message});}
    }finally{if(seq===requestSequence)pending=null;}
  }
  function closePanel(){pane.classList.remove('open');toggle.setAttribute('aria-expanded','false');}
  $('organize').onclick=organize;
  $('pause-organize').onclick=()=>organizationAbort.abort();
  retry.onclick=()=>sync(true);
  toggle.onclick=()=>{const open=pane.classList.toggle('open');toggle.setAttribute('aria-expanded',String(open));updatePanelBounds();if(open){sync(true);taskCanvas.refresh();}};
  $('close').onclick=()=>{closePanel();toggle.focus();};
  $('pending-tray').addEventListener('toggle',e=>{if(e.target.classList?.contains('pending-messages'))pendingOpen=e.target.open;},true);
  $('pending-tray').onclick=e=>{
    const page=e.target.closest('[data-pending-page]');if(page){pendingOffset=Number(page.dataset.pendingPage);pendingOpen=true;renderTree();return;}
    const node=e.target.closest('[data-node]');if(node)detail(node.dataset.node);
  };
  $('expand-tree').onclick=()=>{collapsedNodes.clear();renderTree();taskCanvas.fit();};
  $('collapse-tree').onclick=()=>{if(!data)return;const tree=taskTreeView(data.nodes,data.currentNodeId);for(const id of tree.children.keys())collapsedNodes.add(id);renderTree();taskCanvas.fit();};
  $('locate-current').onclick=()=>{
    if(!data?.currentNodeId)return;
    for(const id of taskTreeView(data.nodes,data.currentNodeId).activePath)collapsedNodes.delete(id);
    tab('map');renderTree();taskCanvas.focus(data.currentNodeId);
  };
  $('fit-canvas').onclick=()=>taskCanvas.fit();
  $('zoom-in').onclick=()=>taskCanvas.zoom(1.25);$('zoom-out').onclick=()=>taskCanvas.zoom(.8);$('canvas-zoom').onclick=()=>taskCanvas.reset();
  for(const [key,delta] of [['source-prev',-1],['source-next',1]])$(key).onclick=()=>{if(sourceState.focusId)sourceState.part+=delta;else sourceState.page+=delta;renderTranscript();};
  $('source-list').onclick=()=>{sourceState.focusId=null;renderTranscript();};
  $('transcript').onclick=e=>{const target=e.target.closest('[data-source-full]');if(target)showSource(target.dataset.sourceFull);};
  $('map-tab').onclick=()=>tab('map');$('source-tab').onclick=()=>tab('source');$('close-detail').onclick=()=>{$('details').hidden=true;};
  const observer=new MutationObserver(scheduleMount);observer.observe(document.body,{childList:true,subtree:true});
  const onEscape=e=>{if(e.key==='Escape'&&pane.classList.contains('open')){if(!settingsForm.hidden){settingsForm.hidden=true;$('api-key').value='';}else if(!$('details').hidden)$('details').hidden=true;else{closePanel();toggle.focus();}}};
  window.addEventListener('resize',updatePanelBounds);window.addEventListener('keydown',onEscape);
  const timer=setInterval(()=>{const hidden=!currentId();if(toggle.hidden!==hidden)toggle.hidden=hidden;if(pane.classList.contains('open')){sync();if(hidden)closePanel();}},1500);
  mountEntry();
  window.__conversationCanvasCleanup=()=>{disposed=true;boundsObserver.disconnect();taskCanvas.dispose();activeApiTester?.dispose();externalOrganizer.dispose();historyAbort?.abort();organizationAbort.abort();requestSequence++;clearInterval(timer);observer.disconnect();cancelAnimationFrame(mountFrame);window.removeEventListener('resize',updatePanelBounds);window.removeEventListener('keydown',onEscape);toggle.remove();ownedGroup?.remove();entryStyle.remove();host.remove();};
})();
