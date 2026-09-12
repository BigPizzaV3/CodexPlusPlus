import {parseMessage,buildGraph} from './model.mjs';

export function graphFromExport(result, threadId, annotations={nodes:[]}) {
  if(result?.status!=='ok')throw new Error(result?.message||'Codex++ 会话读取失败');
  if(result.kind!=='codex-rollout'||result.session_id!==threadId||typeof result.content!=='string')throw new Error('会话接口返回了不匹配的数据');
  const messages=[],seen=new Set();
  let declaredId=null;
  // Ignore an unfinished JSONL tail; the next refresh will read the completed line.
  const lines=result.content.slice(0,result.content.lastIndexOf('\n')+1).split('\n');
  for(let i=0;i<lines.length;i++){
    if(!lines[i].trim())continue;
    let r;try{r=JSON.parse(lines[i]);}catch{continue;}
    if(r.type==='session_meta')declaredId=r.payload?.id;
    const m=parseMessage(r,`message-${threadId}-line-${i}`);
    if(m&&!seen.has(m.id)){seen.add(m.id);messages.push(m);}
  }
  if(declaredId&&declaredId!==threadId)throw new Error('记录中的任务 ID 与当前任务不一致');
  return {threadId,title:annotations.title||'当前对话',messages,...buildGraph(messages,annotations)};
}
