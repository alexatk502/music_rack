import { chromium } from 'playwright-core';
const shell = '/home/atkinsona/.cache/ms-playwright/chromium_headless_shell-1223/chrome-headless-shell-linux64/chrome-headless-shell';
const browser = await chromium.launch({ executablePath: shell, args: ['--no-sandbox','--autoplay-policy=no-user-gesture-required'] });
const page = await browser.newPage({ viewport:{width:1280,height:760} });
const errors=[]; page.on('pageerror',(e)=>errors.push(e.message));
await page.goto('http://127.0.0.1:8123/app.html');
await page.evaluate(() => localStorage.clear());
await page.reload();
await page.waitForFunction(()=>window.__rackState==='off',{timeout:35000});

// Power on so egui repaints continuously and __rackModules stays current.
for (const x of [120,150,180,210,240,270]){ await page.mouse.click(x,18); try{await page.waitForFunction(()=>window.__rackState!=='off',{timeout:1500});break;}catch{} }
await page.waitForFunction(()=>window.__rackState==='patched',{timeout:35000});

const mods = async () => { await page.waitForTimeout(250); return page.evaluate(()=>window.__rackModules); };
const m0 = await mods();
console.log('initial modules:', m0);

const selectVco = async () => { await page.mouse.click(180, 40); await page.waitForTimeout(150); };

await selectVco();
await page.keyboard.press('Control+d');
const mDup = await mods();
console.log('after Ctrl+D duplicate:', mDup);
if (mDup !== m0 + 1) throw new Error(`duplicate failed: ${m0} -> ${mDup}`);

await page.keyboard.press('Control+z');
const mUndo = await mods();
console.log('after Ctrl+Z undo:', mUndo);
if (mUndo !== m0) throw new Error(`undo failed: ${mUndo}`);

await page.keyboard.press('Control+y');
const mRedo = await mods();
console.log('after Ctrl+Y redo:', mRedo);
if (mRedo !== m0 + 1) throw new Error(`redo failed: ${mRedo}`);

await selectVco();
await page.keyboard.press('Control+c');
await page.waitForTimeout(100);
await page.keyboard.press('Control+v');
const mPaste = await mods();
console.log('after Ctrl+C/Ctrl+V paste:', mPaste);
if (mPaste !== m0 + 2) throw new Error(`paste failed: ${mPaste}`);

await page.keyboard.press('Shift+Slash');
await page.waitForTimeout(200);
await page.keyboard.press('Escape');
await page.waitForTimeout(150);

console.log('state:', await page.evaluate(()=>window.__rackState), '| errors:', errors.length);
if (errors.length) throw new Error('page errors: ' + errors.slice(0,3).join(' | '));
console.log('QOL browser test PASSED');
process.exit(0);
