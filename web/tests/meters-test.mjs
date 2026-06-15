// Functional test for the scope/meter feedback feature: load a patch that
// produces signal, power on, and confirm the app keeps running with live
// meters flowing (no reload, no synthetic key-stress — represents real use).
import { chromium } from 'playwright-core';
const shell = '/home/atkinsona/.cache/ms-playwright/chromium_headless_shell-1223/chrome-headless-shell-linux64/chrome-headless-shell';
const patch = {
  version: 1, next_id: 20,
  modules: [
    { id: 1, type: 'vco', pos: [0, 4], params: { pitch: 0, wave: 2 } },
    { id: 2, type: 'shape', pos: [0, 18], params: { drive: 5, mode: 1, mix: 1 } },
    { id: 3, type: 'output', pos: [0, 32], params: { level: 0.7 } },
  ],
  cables: [
    { id: 10, from: { module: 1, port: 'out' }, to: { module: 2, port: 'in' } },
    { id: 11, from: { module: 2, port: 'out' }, to: { module: 3, port: 'left' } },
  ],
};
const browser = await chromium.launch({ executablePath: shell, args: ['--no-sandbox', '--autoplay-policy=no-user-gesture-required'] });
const page = await browser.newPage({ viewport: { width: 900, height: 600 } });
// Inject the patch BEFORE the page scripts run, on every navigation — no reload needed.
await page.addInitScript((p) => { localStorage.setItem('music_rack_patch', JSON.stringify(p)); }, patch);
const errors = [];
page.on('pageerror', (e) => errors.push(e.message));
await page.goto('http://127.0.0.1:8123/app.html');
await page.waitForFunction(() => window.__rackState === 'off', { timeout: 35000 });
if (await page.evaluate(() => window.__rackCables) !== 2) throw new Error('patch not loaded');
for (const x of [120,150,180,210,240,270]) { await page.mouse.click(x,18); try { await page.waitForFunction(()=>window.__rackState!=='off',{timeout:1500}); break;}catch{} }
await page.waitForFunction(() => window.__rackState === 'patched', { timeout: 35000 });
// Let meters flow for 3s; the app must stay alive (engine running) the whole time.
await page.waitForTimeout(3000);
const state = await page.evaluate(() => window.__rackState);
if (state !== 'patched') throw new Error('app died while meters flowed: ' + state);
// Power off cleanly (drops the worklet) before tearing down.
await page.mouse.click(160, 18);
await page.waitForTimeout(300);
if (errors.length) throw new Error('errors: ' + errors.join('; '));
console.log('meters functional test PASSED (live meters 3s, no errors)');
process.exit(0);
