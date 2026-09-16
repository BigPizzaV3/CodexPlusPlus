import {visibleTreeRows} from './model.mjs';

export const canvasNodeSize={width:268,height:132,gapX:88,gapY:34};

// Iterative preorder/reverse traversal keeps deep histories off the JS call stack.
export function layoutTaskCanvas(nodes,currentNodeId,collapsed=new Set()){
  const view=visibleTreeRows(nodes,currentNodeId,collapsed),children=new Map(),positions=new Map();
  const {width,height,gapX,gapY}=canvasNodeSize;
  for(const row of view.rows){const list=children.get(row.node.parent)||[];list.push(row.node.id);children.set(row.node.parent,list);}
  let leaf=0;
  for(const row of view.rows)if(!children.has(row.node.id))positions.set(row.node.id,leaf++*(height+gapY));
  for(let i=view.rows.length-1;i>=0;i--){const row=view.rows[i],list=children.get(row.node.id);if(list)positions.set(row.node.id,(positions.get(list[0])+positions.get(list.at(-1)))/2);}
  const cards=view.rows.map(row=>({...row,x:row.depth*(width+gapX),y:positions.get(row.node.id),width,height}));
  const byId=new Map(cards.map(card=>[card.node.id,card]));
  const edges=cards.filter(card=>byId.has(card.node.parent)).map(card=>({from:byId.get(card.node.parent),to:card,active:card.active&&byId.get(card.node.parent).active}));
  return {cards,byId,edges,pending:view.pending,width:cards.reduce((v,c)=>Math.max(v,c.x+c.width),0),height:cards.reduce((v,c)=>Math.max(v,c.y+c.height),0)};
}

export function fitCanvas(layout,width,height){
  if(!layout.width||!width||!height)return {x:40,y:40,scale:1};
  const scale=Math.max(.00002,Math.min(1,(Math.max(100,width)-80)/layout.width,(Math.max(100,height)-80)/layout.height));
  return {x:(width-layout.width*scale)/2,y:(height-layout.height*scale)/2,scale};
}
export function zoomCanvas(camera,scale,x,y){
  scale=Math.max(.00002,Math.min(2.5,scale));
  return {x:x-(x-camera.x)*scale/camera.scale,y:y-(y-camera.y)*scale/camera.scale,scale};
}
export function canvasViewport(layout,camera,width,height,limit=600){
  const left=(-camera.x-120)/camera.scale,top=(-camera.y-120)/camera.scale,right=(width-camera.x+120)/camera.scale,bottom=(height-camera.y+120)/camera.scale;
  const cards=[];let total=0;
  for(const card of layout.cards)if(card.x+card.width>=left&&card.x<=right&&card.y+card.height>=top&&card.y<=bottom){total++;if(cards.length<limit)cards.push(card);}
  const edges=[];
  for(const edge of layout.edges){const a=edge.from,b=edge.to;if(b.x>=left&&a.x+a.width<=right&&Math.max(a.y,b.y)+a.height/2>=top&&Math.min(a.y,b.y)+a.height/2<=bottom){edges.push(edge);if(edges.length>=2400)break;}}
  return {cards,edges,total};
}

export function canvasCardsMarkup(cards,collapsed,selected,currentNodeId){
  const escape=value=>String(value??'').replace(/[&<>"']/g,c=>({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[c]));
  return cards.map(({node:n,x,y,width,height,childCount,active})=>`<div class="canvas-card-wrap" style="left:${x}px;top:${y}px;width:${width}px;height:${height}px"><button class="node canvas-card ${n.id===currentNodeId?'current':''} ${selected===n.id?'selected':''} ${active?'on-path':''}" data-node="${escape(n.id)}" title="${escape(n.title)}"><small>${escape(n.status)}${n.id===currentNodeId&&n.status!=='当前推进'?' · 当前推进':''}</small><strong>${escape(n.title)}</strong><p>${escape(n.summary)}</p><span class="canvas-card-meta">${childCount?`${childCount} 个分支`:'任务节点'} · ${n.sources?.length||0} 条依据</span></button>${childCount?`<button class="canvas-fold" data-collapse="${escape(n.id)}" aria-label="${collapsed.has(n.id)?'展开':'收起'} ${escape(n.title)}" aria-expanded="${!collapsed.has(n.id)}">${collapsed.has(n.id)?'+':'−'}</button>`:''}</div>`).join('');
}
