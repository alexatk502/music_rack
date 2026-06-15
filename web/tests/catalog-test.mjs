import { chromium } from 'playwright-core';
const shell = '/home/atkinsona/.cache/ms-playwright/chromium_headless_shell-1223/chrome-headless-shell-linux64/chrome-headless-shell';
// FM osc → bitcrush → compressor → pan → output, with a Euclidean clock and
// envelope follower in the mix. Exercises several new module types at once.
const patch = { version:1, next_id:80, modules:[
  {id:1,type:'clock',pos:[0,2],params:{bpm:120,width:0.5}},
  {id:2,type:'euclid',pos:[0,16],params:{length:15,fill:5,rotate:0}},
  {id:3,type:'fmop',pos:[0,30],params:{pitch:0,ratio:3,index:4,feedback:0.2}},
  {id:4,type:'bitcrush',pos:[0,44],params:{bits:6,downsample:2,mix:0.8}},
  {id:5,type:'compressor',pos:[1,2],params:{threshold:-20,ratio:4,attack:0.01,release:0.1,makeup:3}},
  {id:6,type:'pan',pos:[1,16],params:{pan:0.2}},
  {id:7,type:'output',pos:[1,30],params:{level:0.6}},
  {id:8,type:'additive',pos:[1,44],params:{pitch:0.75,partials:10,rolloff:1.2,'odd/even':0.4}},
  {id:9,type:'comb',pos:[2,2],params:{pitch:0,decay:0.98,damp:0.3}},
  {id:10,type:'maths',pos:[2,16],params:{rise:0.05,fall:0.3,cycle:1}},
  {id:11,type:'vcabank',pos:[2,30],params:{}},
  {id:12,type:'turing',pos:[2,52],params:{length:8,prob:0.4}}],
  cables:[
    {id:20,from:{module:1,port:'out'},to:{module:2,port:'clock'}},
    {id:21,from:{module:3,port:'out'},to:{module:4,port:'in'}},
    {id:22,from:{module:4,port:'out'},to:{module:5,port:'in'}},
    {id:23,from:{module:5,port:'out'},to:{module:6,port:'in'}},
    {id:24,from:{module:6,port:'left'},to:{module:7,port:'left'}},
    {id:25,from:{module:6,port:'right'},to:{module:7,port:'right'}}]};
const browser = await chromium.launch({ executablePath: shell, args: ['--no-sandbox','--autoplay-policy=no-user-gesture-required'] });
const page = await browser.newPage({ viewport:{width:1280,height:760} });
await page.addInitScript((p)=>localStorage.setItem('music_rack_patch',JSON.stringify(p)), patch);
const errors=[]; page.on('pageerror',(e)=>errors.push(e.message));
await page.goto('http://127.0.0.1:8123/app.html');
await page.waitForFunction(()=>window.__rackState==='off',{timeout:35000});
const cables=await page.evaluate(()=>window.__rackCables);
console.log('catalog patch loaded:', cables, 'cables');
for (const x of [120,150,180,210,240,270]){ await page.mouse.click(x,18); try{await page.waitForFunction(()=>window.__rackState!=='off',{timeout:1500});break;}catch{} }
await page.waitForFunction(()=>window.__rackState==='patched',{timeout:35000});
await page.waitForTimeout(3000);
const state=await page.evaluate(()=>window.__rackState);
console.log('state after 3s:', state, '| errors:', errors.length);
if (errors.length) console.log('ERR:', errors.slice(0,2).join(' | '));
await page.mouse.click(160,18); // clean power-off
await page.waitForTimeout(300);
process.exit(0);
