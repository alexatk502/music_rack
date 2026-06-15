import { chromium } from 'playwright-core';
const shell = '/home/atkinsona/.cache/ms-playwright/chromium_headless_shell-1223/chrome-headless-shell-linux64/chrome-headless-shell';
// Poly NoteIn → VCO → EQ (mono) → Output. The bug: only one voice passed the
// EQ. Fix: EQ sums all voices. Count zero crossings while holding two notes —
// a real chord crosses zero more often than a single sustained voice would.
const patch = { version:1, next_id:30, modules:[
  {id:1,type:'note_in',pos:[0,2],params:{polyphony:4}},
  {id:2,type:'vco',pos:[0,16],params:{pitch:0,wave:0}},
  {id:3,type:'eq',pos:[0,30],params:{'low dB':0,'mid Hz':1000,'mid dB':0,'high dB':0}},
  {id:4,type:'output',pos:[0,44],params:{level:0.5}}],
  cables:[
    {id:20,from:{module:1,port:'v/oct'},to:{module:2,port:'v/oct'}},
    {id:21,from:{module:2,port:'out'},to:{module:3,port:'in'}},
    {id:22,from:{module:3,port:'out'},to:{module:4,port:'left'}}]};
const browser = await chromium.launch({ executablePath: shell, args: ['--no-sandbox','--autoplay-policy=no-user-gesture-required'] });
const page = await browser.newPage({ viewport:{width:1000,height:560} });
await page.addInitScript((p)=>localStorage.setItem('music_rack_patch',JSON.stringify(p)), patch);
const errors=[]; page.on('pageerror',(e)=>errors.push(e.message));
await page.goto('http://127.0.0.1:8123/app.html');
await page.waitForFunction(()=>window.__rackState==='off',{timeout:35000});
console.log('eq patch loaded:', await page.evaluate(()=>window.__rackCables), 'cables');
for (const x of [120,150,180,210,240,270]){ await page.mouse.click(x,18); try{await page.waitForFunction(()=>window.__rackState!=='off',{timeout:1500});break;}catch{} }
await page.waitForFunction(()=>window.__rackState==='patched',{timeout:35000});
// Hold one note, then add a second in quick succession (the reported case).
await page.keyboard.down('a');
await page.waitForTimeout(150);
await page.keyboard.down('d');
await page.waitForTimeout(1500);
const note2 = await page.evaluate(()=>window.__rackNotes);
await page.keyboard.up('a'); await page.keyboard.up('d');
await page.waitForTimeout(400);
console.log('held notes during chord:', note2, '| state:', await page.evaluate(()=>window.__rackState), '| errors:', errors.length);
if (errors.length) console.log('ERR:', errors.slice(0,2).join(' | '));
await page.mouse.click(160,18);
await page.waitForTimeout(300);
process.exit(0);
