function createTaskCanvas(viewport,{onNode,onCollapse,onCamera}){
  viewport.innerHTML='<svg class="canvas-links" aria-hidden="true"><g></g></svg><div class="canvas-world"></div><div class="canvas-empty"><strong>让任务与尝试形成脉络</strong><p>点击「整理脉络」，将对话归纳为可追溯的任务树。</p></div><div class="canvas-density" role="status" hidden></div>';
  const world=viewport.querySelector('.canvas-world'),links=viewport.querySelector('g'),empty=viewport.querySelector('.canvas-empty'),density=viewport.querySelector('.canvas-density');
  let layout=layoutTaskCanvas([],null),collapsed=new Set(),selected=null,current=null,camera={x:40,y:40,scale:1},context='',needsFit=true,frame=0,cardKey='',pointer=null,suppressClick=false;
  const cameras=new Map(),events=new AbortController();
  function queue(){if(!frame)frame=requestAnimationFrame(draw);}
  function draw(){
    frame=0;const width=viewport.clientWidth,height=viewport.clientHeight;if(!width||!height)return;
    if(needsFit&&layout.cards.length){camera=fitCanvas(layout,width,height);needsFit=false;}
    const visible=canvasViewport(layout,camera,width,height),transform=`translate(${camera.x}px,${camera.y}px) scale(${camera.scale})`;
    world.style.transform=transform;links.setAttribute('transform',`translate(${camera.x},${camera.y}) scale(${camera.scale})`);
    const key=JSON.stringify([selected,current,visible.cards.map(c=>[c.node.id,c.x,c.y,collapsed.has(c.node.id)])]);
    if(cardKey!==key){world.innerHTML=canvasCardsMarkup(visible.cards,collapsed,selected,current);cardKey=key;}
    links.innerHTML=visible.edges.map(({from:a,to:b,active})=>{const x=a.x+a.width,y=a.y+a.height/2,endX=b.x,endY=b.y+b.height/2,mid=(x+endX)/2;return `<path class="${active?'on-path':''}" d="M ${x} ${y} C ${mid} ${y}, ${mid} ${endY}, ${endX} ${endY}"/>`;}).join('');
    empty.hidden=layout.cards.length>0;density.hidden=visible.total<=visible.cards.length;density.textContent=`当前视野包含 ${visible.total} 个节点，放大后查看全部细节。`;
    viewport.style.backgroundSize=`${Math.max(8,24*camera.scale)}px ${Math.max(8,24*camera.scale)}px`;viewport.style.backgroundPosition=`${camera.x}px ${camera.y}px`;
    onCamera(camera);if(context)cameras.set(context,{...camera});
  }
  function zoom(factor,x=viewport.clientWidth/2,y=viewport.clientHeight/2){camera=zoomCanvas(camera,camera.scale*factor,x,y);needsFit=false;queue();}
  function fit(){needsFit=true;queue();}
  function focus(id){const card=layout.byId.get(id);if(!card)return;const scale=Math.max(.75,Math.min(1.2,camera.scale));camera={scale,x:viewport.clientWidth/2-(card.x+card.width/2)*scale,y:viewport.clientHeight/2-(card.y+card.height/2)*scale};needsFit=false;queue();}
  const listen=(target,event,handler,options={})=>target.addEventListener(event,handler,{...options,signal:events.signal});
  listen(viewport,'wheel',event=>{event.preventDefault();const r=viewport.getBoundingClientRect();const delta=event.deltaY*(event.deltaMode===1?16:event.deltaMode===2?viewport.clientHeight:1);zoom(Math.exp(-Math.max(-300,Math.min(300,delta))*.003),event.clientX-r.left,event.clientY-r.top);},{passive:false});
  listen(viewport,'pointerdown',event=>{
    suppressClick=false;
    if(event.button!==0||event.target.closest('[data-collapse]'))return;
    pointer={id:event.pointerId,x:event.clientX,y:event.clientY,camera:{...camera},moved:false};
    if(!event.target.closest('button'))viewport.focus({preventScroll:true});
  });
  listen(viewport,'pointermove',event=>{
    if(!pointer||event.pointerId!==pointer.id)return;
    const dx=event.clientX-pointer.x,dy=event.clientY-pointer.y;
    if(!pointer.moved&&Math.hypot(dx,dy)<5)return;
    if(!pointer.moved){pointer.moved=true;viewport.setPointerCapture(event.pointerId);viewport.classList.add('panning');}
    event.preventDefault();needsFit=false;camera={...pointer.camera,x:pointer.camera.x+dx,y:pointer.camera.y+dy};queue();
  });
  function end(event){if(!pointer||event.pointerId!==pointer.id)return;suppressClick=pointer.moved;pointer=null;viewport.classList.remove('panning');if(viewport.hasPointerCapture(event.pointerId))viewport.releasePointerCapture(event.pointerId);}
  listen(viewport,'pointerup',end);listen(viewport,'pointercancel',end);listen(viewport,'lostpointercapture',()=>{pointer=null;viewport.classList.remove('panning');});
  listen(viewport,'click',event=>{
    if(suppressClick){suppressClick=false;event.preventDefault();event.stopImmediatePropagation();return;}
    const fold=event.target.closest('[data-collapse]');if(fold){onCollapse(fold.dataset.collapse);return;}
    const node=event.target.closest('[data-node]');if(node)onNode(node.dataset.node);
  },{capture:true});
  listen(viewport,'keydown',event=>{
    if(event.target!==viewport)return;
    if(['+','=','-','0','ArrowLeft','ArrowRight','ArrowUp','ArrowDown'].includes(event.key))event.preventDefault();else return;
    if(event.key==='0')fit();else if(event.key==='+'||event.key==='=')zoom(1.25);else if(event.key==='-')zoom(.8);else{camera.x+=event.key==='ArrowLeft'?80:event.key==='ArrowRight'?-80:0;camera.y+=event.key==='ArrowUp'?80:event.key==='ArrowDown'?-80:0;needsFit=false;queue();}
  });
  const resize=new ResizeObserver(queue);resize.observe(viewport);
  return {
    update(nodes,currentId,folded,selection){collapsed=folded;selected=selection;current=currentId;layout=layoutTaskCanvas(nodes,current,collapsed);cardKey='';queue();return layout;},
    select(id){selected=id;cardKey='';queue();},
    context(id){if(id===context)return;if(context&&!needsFit)cameras.set(context,{...camera});context=id;camera=cameras.get(id)||{x:40,y:40,scale:1};needsFit=!cameras.has(id);this.update([],null,new Set(),null);},
    fit,focus,zoom,refresh:queue,
    reset(){camera=zoomCanvas(camera,1,viewport.clientWidth/2,viewport.clientHeight/2);needsFit=false;queue();},
    dispose(){events.abort();resize.disconnect();cancelAnimationFrame(frame);}
  };
}
