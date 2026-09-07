import {parseMessage} from './model.mjs';

export function isExportSizeError(message){
  return /超过.*(?:大小|限制)|too (?:large|big)|size.{0,30}limit|maximum.{0,20}size/i.test(String(message));
}

// Wire contracts verified in the installed Desktop's history loader:
// thread/turns/list(itemsView=notLoaded), then thread/items/list per turn.
export async function readNativeHistory(send,threadId,cache=new Map(),onProgress=()=>{},signal){
  const turns=[],turnIds=new Set(),turnCursors=new Set();
  let cursor=null;
  do{
    signal?.throwIfAborted();
    if(turnCursors.has(cursor))throw Error('历史轮次分页游标重复');
    turnCursors.add(cursor);
    const page=await send('thread/turns/list',{threadId,cursor,limit:20,itemsView:'notLoaded',sortDirection:'asc'});
    if(!Array.isArray(page?.data))throw Error('历史轮次接口返回格式无效');
    for(const turn of page.data){
      if(typeof turn.id!=='string')throw Error('历史轮次缺少 ID');
      if(!turnIds.has(turn.id)){turnIds.add(turn.id);turns.push(turn);}
    }
    cursor=page.nextCursor??null;
    onProgress(`正在读取轮次目录 · ${turns.length} 轮…`);
  }while(cursor!==null);
  const messages=[],seen=new Set();
  for(const [index,turn] of turns.entries()){
    signal?.throwIfAborted();onProgress(`正在读取历史 ${index+1}/${turns.length}…`);
    const key=`${threadId}:${turn.id}`,cached=await cache.get(key);
    let parsed;
    if(cached&&turn.status==='completed')parsed=cached;
    else{
      const partial=turn.status==='completed'?await cache.get(`${key}:partial`):null;
      parsed=partial?.messages?.slice()||[];let itemCursor=partial?.cursor??null,limit=32;const cursors=new Set();
      do{
        signal?.throwIfAborted();
        if(cursors.has(itemCursor))throw Error('历史消息分页游标重复');
        let page;
        for(;;){
          try{page=await send('thread/items/list',{threadId,turnId:turn.id,cursor:itemCursor,limit,sortDirection:'asc'});break;}
          catch(error){
            if(limit>1&&/decoded message length too large/i.test(error.message)){limit=Math.max(1,Math.floor(limit/2));continue;}
            throw error;
          }
        }
        if(!Array.isArray(page?.data))throw Error('历史消息接口返回格式无效');
        cursors.add(itemCursor);
        for(const row of page.data){
          if(row.turnId!=null&&row.turnId!==turn.id)throw Error('历史消息所属轮次不匹配');
          const item=row.item??row;
          if(!['userMessage','agentMessage'].includes(item.type))continue;
          if(typeof item.id!=='string')throw Error('历史消息缺少稳定 ID');
          const role=item.type==='userMessage'?'user':'assistant';
          const content=role==='user'?(item.content??[]):[{text:item.text??''}];
          const message=parseMessage({type:'response_item',payload:{type:'message',id:item.id,role,phase:role==='assistant'?(item.phase??'final_answer'):'',content}},item.id);
          if(message)parsed.push({...message,turnId:turn.id,turnStatus:turn.status});
        }
        itemCursor=page.nextCursor??null;
        onProgress(`正在读取历史 ${index+1}/${turns.length} · 已读 ${parsed.length} 条消息…`);
        if(turn.status==='completed'&&itemCursor!==null)await cache.set(`${key}:partial`,{messages:parsed,cursor:itemCursor});
      }while(itemCursor!==null);
      if(turn.status==='completed'){await cache.set(key,parsed);await cache.delete(`${key}:partial`);}
    }
    // Older completed-turn caches contain text and IDs but no turn metadata.
    // The current directory and cache key provide the authoritative owner.
    for(const message of parsed)if(!seen.has(message.id)){seen.add(message.id);messages.push({...message,turnId:turn.id,turnStatus:turn.status});}
  }
  // Bound cross-task retention, without truncating the returned conversation.
  while(cache.size>500)cache.delete(cache.keys().next().value);
  return messages;
}
