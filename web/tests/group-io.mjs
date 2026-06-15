import { chromium } from 'playwright-core';
import fs from 'fs';
const LOG='/home/atkinsona/projects/music_rack/web/tests/ioresult.txt';
const out=(s)=>{ try{fs.appendFileSync(LOG,s+'\n');}catch{} };
fs.writeFileSync(LOG,'start\n');
process.on('unhandledRejection',e=>{ out('REJECT: '+(e&&e.message||e)); });
try {
const shell='/home/atkinsona/.cache/ms-playwright/chromium_headless_shell-1223/chrome-headless-shell-linux64/chrome-headless-shell';
// A patch with a pre-made collapsed group exposing VCO's (unconnected) v/oct
// input and VCF's out — exercising hand-picked exposure + box rendering.
const patch = {
  version:1, next_id:20,
  modules:[
    {id:1,type:'vco',pos:[0,4],params:{}},
    {id:2,type:'vcf',pos:[0,16],params:{}},
    {id:3,type:'output',pos:[0,40],params:{level:0.5}}],
  cables:[
    {id:10,from:{module:1,port:'out'},to:{module:2,port:'in'}},
    {id:11,from:{module:2,port:'out'},to:{module:3,port:'left'}}],
  groups:[
    {id:12,name:'Tone',members:[1,2],collapsed:true,pos:[0,4],
     exposed_in:[{module:1,port:'v/oct'}],
     exposed_out:[{module:2,port:'out'}]}]
};
const b = await chromium.launch({ executablePath: shell, args:['--no-sandbox','--autoplay-policy=no-user-gesture-required'] });
const p = await b.newPage({ viewport:{width:1100,height:600} });
const errs=[]; p.on('pageerror',e=>errs.push(e.message));
await p.addInitScript((pt)=>localStorage.setItem('music_rack_patch',JSON.stringify(pt)), patch);
await p.goto('http://127.0.0.1:8123/app.html');
await p.waitForFunction(()=>window.__rackState==='off',{timeout:40000});
const g = await p.evaluate(()=>JSON.parse(localStorage.getItem('music_rack_patch')).groups);
out('loaded groups: '+JSON.stringify(g.map(x=>({n:x.name,m:x.members.length,ein:x.exposed_in.length,eout:x.exposed_out.length,c:x.collapsed}))));
for (const x of [120,150,180,210,240,270]){ await p.mouse.click(x,18); try{await p.waitForFunction(()=>window.__rackState!=='off',{timeout:1500});break;}catch{} }
await p.waitForFunction(()=>window.__rackState==='patched',{timeout:40000});
await p.waitForTimeout(1500);
const st = await p.evaluate(()=>({m:window.__rackModules,c:window.__rackCables,s:window.__rackState}));
out('stat '+JSON.stringify(st));
await p.screenshot({ path:'/home/atkinsona/projects/music_rack/web/tests/io.png' });
out('errors '+errs.length+(errs.length?' :: '+errs.slice(0,2).join(' | '):''));
await b.close();
out('done');
} catch(e){ out('THROW: '+(e&&e.message?e.message.split('\n')[0]:e)); }
process.exit(0);
