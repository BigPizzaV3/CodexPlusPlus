let canvasDatabase;
function openCanvasDatabase(){
  if(!canvasDatabase)canvasDatabase=new Promise((resolve,reject)=>{
    const request=indexedDB.open('conversation-canvas',1);
    request.onupgradeneeded=()=>request.result.createObjectStore('records');
    request.onsuccess=()=>resolve(request.result);
    request.onerror=()=>reject(Error('无法打开任务树存储，请检查应用站点存储是否可用'));
    request.onblocked=()=>reject(Error('任务树存储被其他窗口占用，请关闭旧窗口后重试'));
  }).catch(error=>{canvasDatabase=null;throw error;});
  return canvasDatabase;
}
async function canvasStore(key,value,remove=false){
  const db=await openCanvasDatabase(),write=arguments.length>1;
  return new Promise((resolve,reject)=>{
    const tx=db.transaction('records',write?'readwrite':'readonly'),store=tx.objectStore('records');
    const request=remove?store.delete(key):write?store.put(value,key):store.get(key);
    tx.oncomplete=()=>resolve(request.result);
    tx.onerror=tx.onabort=()=>reject(Error('任务树保存失败，可能是存储空间不足；本批进度未确认'));
  });
}
function persistentHistoryCache(){
  const memory=new Map();
  return {async get(key){if(memory.has(key))return memory.get(key);const value=await canvasStore(`history:${key}`);if(value!==undefined)memory.set(key,value);while(memory.size>500)memory.delete(memory.keys().next().value);return value;},
    async set(key,value){await canvasStore(`history:${key}`,value);memory.set(key,value);while(memory.size>500)memory.delete(memory.keys().next().value);},
    async delete(key){memory.delete(key);await canvasStore(`history:${key}`,null,true);}};
}
