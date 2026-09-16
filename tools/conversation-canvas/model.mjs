export function parseMessage(record, fallbackId) {
  const p = record.payload;
  if (record.type !== 'response_item' || p?.type !== 'message' || !['user','assistant'].includes(p.role)) return null;
  let text = (p.content || []).map(c => c.text || '').join('\n').trim();
  if (!text || /^<(recommended_plugins|environment_context)/.test(text) || text.startsWith('# AGENTS.md instructions')) return null;
  if (text.startsWith('<send_user_message_question_reply>')) {
    try { text = JSON.parse(text.replace(/<\/?send_user_message_question_reply>/g,'').trim()).map(a=>a.answer).join('\n'); } catch { return null; }
  }
  text = text.replace(/<oai-mem-citation>[\s\S]*?<\/oai-mem-citation>/g,'').trim();
  // Callers supply a stable fallback; native Codex message IDs take precedence.
  const id = p.id || (typeof fallbackId==='function'?fallbackId(text):fallbackId);
  if (!id) throw new Error('Message without an ID requires a stable fallback ID');
  return {id, text, role:p.role, phase:p.phase || '', timestamp:record.timestamp, ordinal:record.ordinal, turnId:p.internal_chat_message_metadata_passthrough?.turn_id || null};
}

export function excerpt(text, length=46) {
  return text.replace(/```[\s\S]*?```/g,'').replace(/[*#`]/g,'').replace(/\s+/g,' ').trim().slice(0,length);
}

export function buildGraph(messages, annotations={nodes:[]}) {
  const ids = new Set(messages.map(m=>m.id));
  const nodes = (annotations.nodes || []).filter(n=>n.sources?.length && n.sources.every(id=>ids.has(id))).map(n=>({...n,summaryKind:'已整理'}));
  const covered = new Set(nodes.flatMap(n=>n.sources));
  for(const id of annotations.incompleteSources||[])covered.delete(id);
  let previous = nodes.filter(n=>n.lane==='main').at(-1)?.id || null;
  for (const m of messages) {
    if (covered.has(m.id) || (m.role==='assistant' && m.phase!=='final_answer')) continue;
    nodes.push({id:`auto-${m.id}`,parent:annotations.schemaVersion===2?null:previous,lane:annotations.schemaVersion===2?'pending':'main',title:excerpt(m.text,34),summary:excerpt(m.text,160),description:m.text, status:'待整理',summaryKind:'原文摘录',sources:[m.id]});
    previous = `auto-${m.id}`;
  }
  const nodeIds=new Set(nodes.map(n=>n.id));
  return {schemaVersion:annotations.schemaVersion||1,currentNodeId:nodeIds.has(annotations.currentNodeId)?annotations.currentNodeId:null,nodes,edges:nodes.filter(n=>n.parent && nodeIds.has(n.parent)).map(n=>({from:n.parent,to:n.id,kind:n.lane==='branch'?'branch':'main'}))};
}

// Build a view index without inferring any new task relationships. Old saved
// graphs may have filtered-out parents; keep surviving nodes visible as roots.
export function taskTreeView(nodes,currentNodeId=null){
  const pending=nodes.filter(n=>n.summaryKind==='原文摘录');
  const ready=nodes.filter(n=>n.summaryKind!=='原文摘录');
  const byId=new Map(ready.map(n=>[n.id,n])),children=new Map(),roots=[];
  for(const node of ready){
    if(node.parent&&byId.has(node.parent)){
      const siblings=children.get(node.parent)||[];siblings.push(node);children.set(node.parent,siblings);
    }else roots.push(node);
  }
  const activePath=new Set();let node=byId.get(currentNodeId);
  while(node&&!activePath.has(node.id)){activePath.add(node.id);node=byId.get(node.parent);}
  return {roots,children,pending,activePath};
}

export function visibleTreeRows(nodes,currentNodeId,collapsed=new Set()){
  const view=taskTreeView(nodes,currentNodeId),rows=[],seen=new Set();
  const stack=view.roots.slice().reverse().map(node=>({node,depth:0}));
  while(stack.length){
    const row=stack.pop();if(seen.has(row.node.id))continue;seen.add(row.node.id);
    const children=view.children.get(row.node.id)||[];
    rows.push({...row,childCount:children.length,active:view.activePath.has(row.node.id)});
    if(!collapsed.has(row.node.id))for(let i=children.length-1;i>=0;i--)stack.push({node:children[i],depth:row.depth+1});
  }
  return {rows,pending:view.pending};
}
