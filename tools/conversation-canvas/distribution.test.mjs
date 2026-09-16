import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import {execFileSync} from 'node:child_process';

test('release builds neither need nor embed local conversation annotations',()=>{
  const temp=fs.mkdtempSync(path.join(os.tmpdir(),'canvas-build-'));
  try{
    const root=new URL('./',import.meta.url);
    for(const name of fs.readdirSync(root))if(name.endsWith('.mjs')&&!name.endsWith('.test.mjs'))fs.copyFileSync(new URL(name,root),path.join(temp,name));
    fs.cpSync(new URL('src/',root),path.join(temp,'src'),{recursive:true});
    execFileSync(process.execPath,['build-userscript.mjs'],{cwd:temp});
    const clean=fs.readFileSync(path.join(temp,'public/canvas.user.js'),'utf8');
    fs.mkdirSync(path.join(temp,'data'));
    fs.writeFileSync(path.join(temp,'data/private.json'),JSON.stringify({title:'private-build-sentinel',nodes:[{summary:'must stay local'}]}));
    execFileSync(process.execPath,['build-userscript.mjs'],{cwd:temp});
    const rebuilt=fs.readFileSync(path.join(temp,'public/canvas.user.js'),'utf8');
    assert.equal(clean,rebuilt);assert.match(rebuilt,/const annotationIndex=\{\};/);
    assert.doesNotMatch(rebuilt,/private-build-sentinel|must stay local/);
  }finally{
    // Remove only the directory allocated by this test, under the OS temp root.
    assert.ok(path.resolve(temp).startsWith(path.resolve(os.tmpdir())+path.sep));
    fs.rmSync(temp,{recursive:true,force:true});
  }
});
