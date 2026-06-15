import { chromium } from 'playwright-core';
const shell = '/home/atkinsona/.cache/ms-playwright/chromium_headless_shell-1223/chrome-headless-shell-linux64/chrome-headless-shell';
const patch = { version:1, next_id:40, modules:[
  {id:1,type:'note_in',pos:[0,2],params:{polyphony:4}},
  {id:2,type:'macro_osc',pos:[0,16],params:{model:4,pitch:0,harmonics:0.6,timbre:0.4,morph:0.3}},
  {id:3,type:'adsr',pos:[0,40],params:{attack:0.01,decay:0.2,sustain:0.7,release:0.3}},
  {id:4,type:'vca',pos:[0,54],params:{}},
  {id:5,type:'output',pos:[0,68],params:{level:0.6}}],
  cables:[
    {id:20,from:{module:1,port:'v/oct'},to:{module:2,port:'v/oct'}},
    {id:21,from:{module:1,port:'gate'},to:{module:3,port:'gate'}},
    {id:22,from:{module:2,port:'main'},to:{module:4,port:'in'}},
    {id:23,from:{module:3,port:'env'},to:{module:4,port:'cv'}},
    {id:24,from:{module:4,port:'out'},to:{module:5,port:'left'}}]};
const browser = await chromium.launch({ executablePath: shell, args: ['--no-sandbox','--autoplay-policy=no-user-gesture-required'] });
const page = await browser.newPage({ viewport:{width:1100,height:600} });
await page.addInitScript((p)=>localStorage.setItem('music_rack_patch',JSON.stringify(p)), patch);
const errors=[]; page.on('pageerror',(e)=>errors.push(e.message));
await page.goto('http://127.0.0.1:8123/app.html');
await page.waitForFunction(()=>window.__rackState==='off',{timeout:35000});
console.log('macro patch loaded:', await page.evaluate(()=>window.__rackCables), 'cables');
for (const x of [120,150,180,210,240,270]){ await page.mouse.click(x,18); try{await page.waitForFunction(()=>window.__rackState!=='off',{timeout:1500});break;}catch{} }
await page.waitForFunction(()=>window.__rackState==='patched',{timeout:35000});
await page.keyboard.down('a'); await page.keyboard.down('g');
await page.waitForTimeout(1200);
await page.keyboard.up('a'); await page.keyboard.up('g');
await page.waitForTimeout(800);
console.log('state:', await page.evaluate(()=>window.__rackState), '| errors:', errors.length);
if (errors.length) console.log('ERR:', errors.slice(0,2).join(' | '));
await page.mouse.click(160,18);
await page.waitForTimeout(300);
process.exit(0);
