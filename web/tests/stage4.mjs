import { chromium } from 'playwright-core';
const shell='/home/atkinsona/.cache/ms-playwright/chromium_headless_shell-1223/chrome-headless-shell-linux64/chrome-headless-shell';
const b = await chromium.launch({ executablePath: shell, args:['--no-sandbox','--autoplay-policy=no-user-gesture-required'] });
const p = await b.newPage({ viewport:{width:1280,height:800} });
const errs=[]; p.on('pageerror',e=>errs.push(e.message));
await p.goto('http://127.0.0.1:8123/app.html');
await p.evaluate(()=>localStorage.clear()); await p.reload();
await p.waitForFunction(()=>window.__rackState==='off',{timeout:40000});
for (const x of [120,150,180,210,240,270]){ await p.mouse.click(x,18); try{await p.waitForFunction(()=>window.__rackState!=='off',{timeout:1500});break;}catch{} }
await p.waitForFunction(()=>window.__rackState==='patched',{timeout:40000});
const grp = async () => { await p.waitForTimeout(1300); return p.evaluate(()=>JSON.parse(localStorage.getItem('music_rack_patch')||'{}').groups||[]); };
const stat = () => p.evaluate(()=>({m:window.__rackModules,c:window.__rackCables,s:window.__rackState}));

console.log('before', JSON.stringify(await stat()));
// Box-select VCO (x~160-270) + VCF (x~300-410): drag from empty (below, between
// VCF and ADSR) up-left across both panels.
await p.mouse.move(425, 360); await p.mouse.down();
for (let i=1;i<=10;i++){ await p.mouse.move(425-(425-165)*i/10, 360-(360-110)*i/10); }
await p.mouse.up(); await p.waitForTimeout(200);
// Group them.
await p.keyboard.press('Control+g');
const g1 = await grp();
console.log('after group: groups='+JSON.stringify(g1.map(g=>({n:g.name,m:g.members.length,c:g.collapsed}))));
if (g1.length !== 1) throw new Error('expected 1 group, got '+g1.length);
if (g1[0].members.length !== 2) throw new Error('expected 2 members, got '+g1[0].members.length);
if (g1[0].collapsed !== true) throw new Error('group should be collapsed');

// Modules and cables are unchanged (grouping is UI-only); engine still runs.
const st = await stat();
console.log('after group stat '+JSON.stringify(st));
if (st.m !== 6) throw new Error('modules changed: '+st.m);
if (st.c !== 7) throw new Error('cables changed: '+st.c);
if (st.s !== 'patched') throw new Error('engine died: '+st.s);

await p.screenshot({ path:'/tmp/mr-test/stage4.png' });
console.log('errors '+errs.length);
if (errs.length) throw new Error('page errors: '+errs.slice(0,2).join(' | '));
console.log('STAGE4 GROUP TEST PASSED');
process.exit(0);
