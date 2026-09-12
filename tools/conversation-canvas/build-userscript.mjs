import fs from 'node:fs';
const root=new URL('./',import.meta.url);
const read=file=>fs.readFileSync(new URL(file,root),'utf8');
const portable=file=>read(file).replace(/^import .*;\r?\n/gm,'').replace(/^export /gm,'');
const output=read('src/canvas-panel.js')
  .replace('/* canvas-model */',()=>portable('model.mjs')+'\n'+portable('tree-markup.mjs')+'\n'+portable('bridge-data.mjs')+'\n'+portable('organize.mjs')+'\n'+portable('long-organizer.mjs')+'\n'+portable('view-pages.mjs')+'\n'+read('src/canvas-store.js')+'\n'+portable('native-history.mjs')+'\n'+read('src/native-sidechat.js'))
  .replace('/* canvas-api */',()=>portable('external-api.mjs')+'\n'+read('src/native-api.js'))
  .replace('/* canvas-interaction */',()=>portable('tree-canvas.mjs')+'\n'+read('src/tree-canvas-view.js'))
  .replace('/* canvas-style */',()=>read('src/tree-canvas.css'))
  // User annotations belong only in local runtime storage, never in a release.
  .replace('/* canvas-annotations */',()=>`const annotationIndex={};`);
fs.mkdirSync(new URL('public/',root),{recursive:true});
fs.writeFileSync(new URL('public/canvas.user.js',root),output.replace(/^[ \t]+$/gm,''));
console.log(`Built native canvas user script ${output.match(/@version\s+(\S+)/)?.[1]}`);
