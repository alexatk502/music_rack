import { chromium } from 'playwright-core';
const shell = '/home/atkinsona/.cache/ms-playwright/chromium_headless_shell-1223/chrome-headless-shell-linux64/chrome-headless-shell';
// A generative patch using the NEW modules: clock → clockdiv → seq, random →
// quantizer → wt-osc → phaser → reverb → output, plus a chorus tap.
const patch = { version:1, next_id:60, modules:[
  {id:1,type:'clock',pos:[0,2],params:{bpm:130,width:0.5}},
  {id:2,type:'clockdiv',pos:[0,16],params:{}},
  {id:3,type:'random',pos:[0,32],params:{rate:6,slew:0.3}},
  {id:4,type:'quantizer',pos:[0,48],params:{scale:3,root:0}},
  {id:5,type:'wtvco',pos:[0,62],params:{pitch:0,position:0.6}},
  {id:6,type:'phaser',pos:[1,2],params:{rate:0.4,depth:0.7,mix:0.5}},
  {id:7,type:'reverb',pos:[1,16],params:{decay:0.8,mix:0.35}},
  {id:8,type:'output',pos:[1,32],params:{level:0.6}},
  {id:9,type:'slew',pos:[1,48],params:{rise:0.2,fall:0.2}},
  {id:10,type:'mult',pos:[1,62],params:{}}],
  cables:[
    {id:20,from:{module:1,port:'out'},to:{module:3,port:'trig'}},
    {id:21,from:{module:3,port:'stepped'},to:{module:4,port:'in'}},
    {id:22,from:{module:4,port:'out'},to:{module:5,port:'v/oct'}},
    {id:23,from:{module:5,port:'out'},to:{module:6,port:'in'}},
    {id:24,from:{module:6,port:'out'},to:{module:7,port:'in'}},
    {id:25,from:{module:7,port:'left'},to:{module:8,port:'left'}},
    {id:26,from:{module:7,port:'right'},to:{module:8,port:'right'}}]};
const browser = await chromium.launch({ executablePath: shell, args: ['--no-sandbox','--autoplay-policy=no-user-gesture-required'] });
const page = await browser.newPage({ viewport:{width:1280,height:720} });
await page.addInitScript((p)=>localStorage.setItem('music_rack_patch',JSON.stringify(p)), patch);
const errors=[]; page.on('pageerror',(e)=>errors.push(e.message));
await page.goto('http://127.0.0.1:8123/app.html');
await page.waitForFunction(()=>window.__rackState==='off',{timeout:35000});
const cables=await page.evaluate(()=>window.__rackCables);
console.log('loaded new-module patch:', cables, 'cables');
for (const x of [120,150,180,210,240,270]){ await page.mouse.click(x,18); try{await page.waitForFunction(()=>window.__rackState!=='off',{timeout:1500});break;}catch{} }
await page.waitForFunction(()=>window.__rackState==='patched',{timeout:35000});
await page.waitForTimeout(3000);
const state=await page.evaluate(()=>window.__rackState);
console.log('state after 3s:', state);
await page.screenshot({ path:'newmods.png' });
await page.mouse.click(160,18); // power off cleanly
await page.waitForTimeout(300);
console.log('errors:', errors.length, errors.slice(0,2).join(' | '));
process.exit(0);
