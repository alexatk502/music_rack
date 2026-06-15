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
const stat=async()=>{await p.waitForTimeout(250);return p.evaluate(()=>({m:window.__rackModules,c:window.__rackCables}));};
const s0=await stat(); console.log('initial '+JSON.stringify(s0));

// Box-select over the VCO (x~160-270) and VCF (x~300-410) panels. Start the
// drag on empty canvas (between VCF and ADSR, below the panels), drag up-left.
await p.mouse.move(425, 360);
await p.mouse.down();
for (let i=1;i<=10;i++){ await p.mouse.move(425-(425-165)*i/10, 360-(360-110)*i/10); }
await p.mouse.up();
await p.waitForTimeout(200);

await p.keyboard.press('Control+c'); await p.waitForTimeout(150);
await p.keyboard.press('Control+v');
const s1=await stat(); console.log('after-paste '+JSON.stringify(s1));
if (s1.m !== s0.m + 2) throw new Error(`expected +2 modules, got ${s0.m}->${s1.m}`);
if (s1.c !== s0.c + 1) throw new Error(`expected +1 internal cable, got ${s0.c}->${s1.c}`);

await p.keyboard.press('Control+z');
const s2=await stat(); console.log('after-undo '+JSON.stringify(s2));
if (s2.m !== s0.m) throw new Error(`undo failed: ${s2.m}`);

console.log('errors '+errs.length);
if (errs.length) throw new Error('page errors: ' + errs.slice(0,2).join(' | '));
console.log('GROUP COPY/PASTE TEST PASSED');
process.exit(0);
