import { chromium } from 'playwright-core';
const shell = '/home/atkinsona/.cache/ms-playwright/chromium_headless_shell-1223/chrome-headless-shell-linux64/chrome-headless-shell';
// NoteIn → Arp → VCO → Ladder → LPG → Saturator → PingPong → Output,
// plus a Tuner watching the VCO and a Voltmeter on the LFO. Exercises many
// of the new modules at once.
const patch = { version:1, next_id:80, modules:[
  {id:1,type:'note_in',pos:[0,2],params:{polyphony:4}},
  {id:2,type:'clock',pos:[0,16],params:{bpm:200,width:0.5}},
  {id:3,type:'arp',pos:[0,30],params:{mode:0,octaves:1,'gate len':0.5}},
  {id:4,type:'vco',pos:[0,46],params:{pitch:0,wave:2}},
  {id:5,type:'ladder',pos:[0,60],params:{cutoff:2,res:0.5,drive:2}},
  {id:6,type:'lpg',pos:[1,2],params:{freq:3,decay:0.2,response:2}},
  {id:7,type:'saturate',pos:[1,16],params:{drive:4,tone:0.6,mix:0.7}},
  {id:8,type:'pingpong',pos:[1,30],params:{time:0.18,feedback:0.4,mix:0.35}},
  {id:9,type:'output',pos:[1,46],params:{level:0.5}},
  {id:10,type:'tuner',pos:[2,2],params:{}},
  {id:11,type:'lfo',pos:[2,18],params:{rate:1,wave:0,bipolar:1}},
  {id:12,type:'voltmeter',pos:[2,32],params:{}},
  {id:13,type:'flanger',pos:[2,46],params:{}},
  {id:14,type:'eq',pos:[2,62],params:{}}],
  cables:[
    {id:20,from:{module:2,port:'out'},to:{module:3,port:'clock'}},
    {id:21,from:{module:3,port:'v/oct'},to:{module:4,port:'v/oct'}},
    {id:22,from:{module:3,port:'gate'},to:{module:6,port:'trig'}},
    {id:23,from:{module:4,port:'out'},to:{module:5,port:'in'}},
    {id:24,from:{module:5,port:'out'},to:{module:6,port:'in'}},
    {id:25,from:{module:6,port:'out'},to:{module:7,port:'in'}},
    {id:26,from:{module:7,port:'out'},to:{module:8,port:'left'}},
    {id:27,from:{module:8,port:'left'},to:{module:9,port:'left'}},
    {id:28,from:{module:8,port:'right'},to:{module:9,port:'right'}},
    {id:29,from:{module:4,port:'out'},to:{module:10,port:'in'}},
    {id:30,from:{module:11,port:'out'},to:{module:12,port:'in'}}]};
const browser = await chromium.launch({ executablePath: shell, args: ['--no-sandbox','--autoplay-policy=no-user-gesture-required'] });
const page = await browser.newPage({ viewport:{width:1280,height:760} });
await page.addInitScript((p)=>localStorage.setItem('music_rack_patch',JSON.stringify(p)), patch);
const errors=[]; page.on('pageerror',(e)=>errors.push(e.message));
await page.goto('http://127.0.0.1:8123/app.html');
await page.waitForFunction(()=>window.__rackState==='off',{timeout:35000});
console.log('batch4 patch loaded:', await page.evaluate(()=>window.__rackCables), 'cables');
for (const x of [120,150,180,210,240,270]){ await page.mouse.click(x,18); try{await page.waitForFunction(()=>window.__rackState!=='off',{timeout:1500});break;}catch{} }
await page.waitForFunction(()=>window.__rackState==='patched',{timeout:35000});
// Hold notes so the arp has something to sequence.
await page.keyboard.down('a'); await page.keyboard.down('d'); await page.keyboard.down('g');
await page.waitForTimeout(2500);
await page.keyboard.up('a'); await page.keyboard.up('d'); await page.keyboard.up('g');
await page.waitForTimeout(500);
console.log('state:', await page.evaluate(()=>window.__rackState), '| errors:', errors.length);
if (errors.length) console.log('ERR:', errors.slice(0,2).join(' | '));
await page.mouse.click(160,18);
await page.waitForTimeout(300);
process.exit(0);
