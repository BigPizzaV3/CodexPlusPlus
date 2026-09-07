export function transcriptPage(messages,state={}){
  const esc=s=>String(s??'').replace(/[&<>"']/g,c=>({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[c]));
  const focus=messages.find(m=>m.id===state.focusId);
  let rows,page,total,info;
  if(focus){
    total=Math.max(1,Math.ceil(focus.text.length/12000));page=Math.max(0,Math.min(state.part||0,total-1));
    const boundary=offset=>offset>0&&offset<focus.text.length&&/[\uDC00-\uDFFF]/.test(focus.text[offset])&&/[\uD800-\uDBFF]/.test(focus.text[offset-1])?offset-1:offset;
    rows=[{...focus,text:focus.text.slice(boundary(page*12000),boundary((page+1)*12000))}];info=`消息全文 · 第 ${page+1}/${total} 段`;
  }else{
    total=Math.max(1,Math.ceil(messages.length/20));page=Math.max(0,Math.min(state.page||0,total-1));
    rows=messages.slice(page*20,page*20+20);info=`消息 ${messages.length?page*20+1:0}–${Math.min(page*20+20,messages.length)} / ${messages.length}`;
  }
  const html=rows.map(m=>`<article class="message ${focus?'highlight':''}" id="source-${esc(m.id)}"><small>${m.role==='user'?'你':'Codex'}${m.phase==='commentary'?' · 进展':''}</small><pre>${esc(focus?m.text:m.text.slice(0,2000))}</pre>${!focus&&m.text.length>2000?`<button data-source-full="${esc(m.id)}">分段查看完整消息（${m.text.length} 字符）</button>`:''}</article>`).join('');
  return {html,info,page,total,focused:!!focus};
}
