import { chromium } from 'playwright-core';
const shell='/home/atkinsona/.cache/ms-playwright/chromium_headless_shell-1223/chrome-headless-shell-linux64/chrome-headless-shell';
const b = await chromium.launch({ executablePath: shell, args:['--no-sandbox','--autoplay-policy=no-user-gesture-required'] });
const p = await b.newPage({ viewport:{width:1280,height:760} });
const errs=[]; p.on('pageerror',e=>errs.push(e.message));
await p.goto('http://127.0.0.1:8123/app.html');
await p.evaluate(()=>localStorage.clear()); await p.reload();
await p.waitForFunction(()=>window.__rackState==='off',{timeout:40000});
for (const x of [120,150,180,210,240,270]){ await p.mouse.click(x,18); try{await p.waitForFunction(()=>window.__rackState!=='off',{timeout:1500});break;}catch{} }
await p.waitForFunction(()=>window.__rackState==='patched',{timeout:40000});
const mods=async()=>{await p.waitForTimeout(250);return p.evaluate(()=>window.__rackModules);};
const m0=await mods(); 
await p.mouse.click(180,40); await p.waitForTimeout(150);
await p.keyboard.press('Control+c'); await p.waitForTimeout(150);
await p.keyboard.press('Control+v'); 
const m1=await mods();
console.log(`COPYPASTE m0=${m0} m1=${m1} errs=${errs.length}`);
process.exit(0);
