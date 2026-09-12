import test from 'node:test';
import assert from 'node:assert/strict';
import {layoutTaskCanvas,fitCanvas,zoomCanvas,canvasViewport,canvasCardsMarkup} from './tree-canvas.mjs';
const node=(id,parent=null)=>({id,parent,title:id,summary:'Summary',status:'待验证',sources:['m1'],summaryKind:'已整理'});
test('canvas lays out siblings on separate rows and centers parents without inventing edges',()=>{
  const layout=layoutTaskCanvas([node('root'),node('a','root'),node('b','root'),node('a1','a'),node('a2','a')],'a2');
  assert.equal(layout.edges.length,4);
  const get=id=>layout.byId.get(id);
  assert.equal(get('a').x,get('b').x);assert.ok(get('b').y>=get('a').y+get('a').height);
  assert.equal(get('a').y,(get('a1').y+get('a2').y)/2);
  assert.ok(get('a1').x>get('a').x+get('a').width);
  assert.deepEqual(layout.edges.filter(e=>e.active).map(e=>e.to.node.id),['a','a2']);
});
test('canvas folds branches and keeps pending messages outside the identified tree',()=>{
  const layout=layoutTaskCanvas([node('root'),node('a','root'),node('a1','a'),{...node('pending'),summaryKind:'原文摘录'}],null,new Set(['a']));
  assert.deepEqual(layout.cards.map(c=>c.node.id),['root','a']);assert.equal(layout.byId.get('a').childCount,1);assert.equal(layout.pending.length,1);
});
test('zoom preserves the world point under the cursor and fit contains a normal tree',()=>{
  const camera={x:100,y:-30,scale:.6},anchor={x:500,y:250},zoomed=zoomCanvas(camera,1.7,anchor.x,anchor.y);
  assert.ok(Math.abs((anchor.x-camera.x)/camera.scale-(anchor.x-zoomed.x)/zoomed.scale)<1e-9);
  assert.ok(Math.abs((anchor.y-camera.y)/camera.scale-(anchor.y-zoomed.y)/zoomed.scale)<1e-9);
  const layout=layoutTaskCanvas([node('r'),node('a','r'),node('b','r')],null),fit=fitCanvas(layout,900,600);
  assert.ok(fit.x>=0&&fit.y>=0);assert.ok(fit.x+layout.width*fit.scale<=900);assert.ok(fit.y+layout.height*fit.scale<=600);
});
test('10000-node chain can reach its tail without stack overflow or rendering the whole history',()=>{
  const nodes=Array.from({length:10000},(_,i)=>node('n'+i,i?'n'+(i-1):null));
  const layout=layoutTaskCanvas(nodes,'n9999'),tail=layout.byId.get('n9999');
  const view=canvasViewport(layout,{x:100-tail.x,y:100,scale:1},900,600);
  assert.ok(view.cards.some(c=>c.node.id==='n9999'));assert.ok(view.cards.length<10);assert.equal(layout.cards.length,10000);
  const fit=fitCanvas(layout,900,600),overview=canvasViewport(layout,fit,900,600);
  assert.ok(overview.cards.length<=600);assert.ok(overview.edges.length<=2400);
});
test('canvas labels escape model text and retain exact source node identity',()=>{
  const n={...node('x" onclick="bad'),title:'<img src=x onerror=bad>'};
  const layout=layoutTaskCanvas([n],n.id);const html=canvasCardsMarkup(layout.cards,new Set(),n.id,n.id);
  assert.ok(!html.includes('<img'));assert.ok(html.includes('&lt;img'));assert.ok(html.includes('data-node="x&quot; onclick=&quot;bad"'));
});
