import { chromium } from 'playwright-core';
const shell = '/home/atkinsona/.cache/ms-playwright/chromium_headless_shell-1223/chrome-headless-shell-linux64/chrome-headless-shell';
const browser = await chromium.launch({ executablePath: shell, args: ['--no-sandbox', '--autoplay-policy=no-user-gesture-required'] });
const page = await browser.newPage({ viewport: { width: 1280, height: 800 } });
const errors = [];
page.on('pageerror', (e) => errors.push(e.message));

const patch = {
  version: 1,
  next_id: 40,
  modules: [
    { id: 1, type: 'clock', pos: [0, 2], params: { bpm: 140, width: 0.4 } },
    { id: 2, type: 'seq8', pos: [0, 16], params: { steps: 8, '1': 0, '2': 0.25, '3': 0.583, '4': 0, '5': 0.917, '6': 0.583, '7': 0.25, '8': -0.417 } },
    { id: 3, type: 'vco', pos: [0, 42], params: { pitch: 0, wave: 2 } },
    { id: 4, type: 'shape', pos: [0, 56], params: { drive: 4, mode: 1, mix: 0.7 } },
    { id: 5, type: 'vcf', pos: [0, 70], params: { cutoff: 2.5, res: 2 } },
    { id: 6, type: 'noise', pos: [1, 2], params: { kind: 1, level: 1 } },
    { id: 7, type: 'snh', pos: [1, 16], params: {} },
    { id: 8, type: 'attn', pos: [1, 30], params: { gain: 0.3, offset: 0 } },
    { id: 9, type: 'vca', pos: [1, 44], params: {} },
    { id: 10, type: 'delay', pos: [1, 58], params: { time: 0.32, feedback: 0.45, mix: 0.35 } },
    { id: 11, type: 'output', pos: [1, 72], params: { level: 0.7 } },
  ],
  cables: [
    { id: 20, from: { module: 1, port: 'out' }, to: { module: 2, port: 'clock' } },
    { id: 21, from: { module: 2, port: 'v/oct' }, to: { module: 3, port: 'v/oct' } },
    { id: 22, from: { module: 3, port: 'out' }, to: { module: 4, port: 'in' } },
    { id: 23, from: { module: 4, port: 'out' }, to: { module: 5, port: 'in' } },
    { id: 24, from: { module: 5, port: 'out' }, to: { module: 9, port: 'in' } },
    { id: 25, from: { module: 2, port: 'gate' }, to: { module: 9, port: 'cv' } },
    { id: 26, from: { module: 9, port: 'out' }, to: { module: 10, port: 'in' } },
    { id: 27, from: { module: 10, port: 'out' }, to: { module: 11, port: 'left' } },
    { id: 28, from: { module: 6, port: 'out' }, to: { module: 7, port: 'in' } },
    { id: 29, from: { module: 1, port: '/4' }, to: { module: 7, port: 'trig' } },
    { id: 30, from: { module: 7, port: 'out' }, to: { module: 8, port: 'in' } },
    { id: 31, from: { module: 8, port: 'out' }, to: { module: 5, port: 'cutoff cv' } },
  ],
};

await page.goto('http://127.0.0.1:8123/app.html');
// Let the first load finish before reloading, or the aborted in-flight wasm
// fetch logs a spurious pageerror.
await page.waitForFunction(() => window.__rackState === 'off', { timeout: 20000 });
await page.evaluate((p) => localStorage.setItem('music_rack_patch', JSON.stringify(p)), patch);
await page.reload();
await page.waitForFunction(() => window.__rackState === 'off', { timeout: 20000 });
const cables = await page.evaluate(() => window.__rackCables);
console.log(`loaded showcase patch: ${cables} cables`);
if (cables !== 12) throw new Error(`expected 12 cables, got ${cables}`);

for (const x of [120, 150, 180, 210, 240, 270]) {
  await page.mouse.click(x, 18);
  try { await page.waitForFunction(() => window.__rackState !== 'off', { timeout: 1500 }); break; } catch {}
}
await page.waitForFunction(() => window.__rackState === 'patched', { timeout: 20000 });
console.log('engine running the generative patch');
await page.waitForTimeout(3000); // let it run a few bars
const state = await page.evaluate(() => window.__rackState);
if (state !== 'patched') throw new Error(`engine died: ${state}`);
await page.screenshot({ path: 'showcase.png' });
if (errors.length) throw new Error(`page errors: ${errors.join('; ')}`);
console.log('showcase browser test PASSED');
await browser.close();
