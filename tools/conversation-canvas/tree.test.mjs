import {test} from 'node:test';
import assert from 'node:assert/strict';
import {buildGraph,taskTreeView} from './model.mjs';
import {treeMarkup} from './tree-markup.mjs';
const make=(id,parent,lane='branch')=>({id,parent,lane,title:id,summary:'概括',description:'详情',status:'待验证',sources:['m1']});
const annotations={schemaVersion:2,currentNodeId:'a11',nodes:[make('root',null,'main'),make('a','root'),make('a1','a'),make('a11','a1'),make('b','root')]};
const messages=[{id:'m1',role:'user',text:'项目目标'},{id:'m2',role:'user',text:'新要求'}];
test('deep tree survives persistence and pending messages do not invent parents',()=>{
  const graph=buildGraph(messages,JSON.parse(JSON.stringify(annotations)));
  const view=taskTreeView(graph.nodes,graph.currentNodeId);
  assert.deepEqual([...view.activePath],['a11','a1','a','root']);
  assert.deepEqual(view.children.get('root').map(n=>n.id),['a','b']);
  assert.equal(view.children.get('a1')[0].id,'a11');
  assert.equal(view.pending[0].id,'auto-m2');assert.equal(view.pending[0].parent,null);
  assert.equal(graph.edges.length,4);
});
test('tree markup omits collapsed descendants and preserves active marker and escaped content when expanded',()=>{
  const graph=buildGraph(messages,annotations);graph.nodes[3].title='<img onerror="bad">';
  const html=treeMarkup(graph.nodes,graph.currentNodeId,new Set(['a']),null);
  for(const id of ['root','a','b'])assert.equal(html.split(`data-node="${id}"`).length-1,1);
  for(const id of ['a1','a11'])assert.equal(html.split(`data-node="${id}"`).length-1,0);
  assert.match(html,/aria-expanded="false"/);
  const expanded=treeMarkup(graph.nodes,graph.currentNodeId);
  assert.match(expanded,/当前推进/);assert.match(expanded,/&lt;img onerror=&quot;bad&quot;&gt;/);assert.doesNotMatch(expanded,/<img/);
  assert.match(html,/待整理消息 · 1 条/);
});
test('legacy one-level saved annotations remain visible and missing sources do not hide valid descendants',()=>{
  const old={nodes:[make('root',null,'main'),make('branch','root')]};
  const graph=buildGraph(messages,old);
  assert.deepEqual(taskTreeView(graph.nodes).roots.map(n=>n.id),['root']);
  old.nodes[0].sources=['missing'];
  const filtered=buildGraph(messages,old);
  assert.deepEqual(taskTreeView(filtered.nodes).roots.map(n=>n.id),['branch']);
});
