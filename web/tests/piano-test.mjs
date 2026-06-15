import { chromium } from 'playwright-core';
const shell = '/home/atkinsona/.cache/ms-playwright/chromium_headless_shell-1223/chrome-headless-shell-linux64/chrome-headless-shell';
const browser = await chromium.launch({ executablePath: shell, args: ['--no-sandbox', '--autoplay-policy=no-user-gesture-required'] });
const page = await browser.newPage({ viewport: { width: 1280, height: 800 } });
const errors = [];
page.on('pageerror', (e) => errors.push(e.message));
await page.goto('http://127.0.0.1:8123/app.html');
await page.waitForFunction(() => window.__rackState === 'off', { timeout: 20000 });
for (const x of [120, 150, 180, 210, 240, 270]) {
  await page.mouse.click(x, 18);
  try { await page.waitForFunction(() => window.__rackState !== 'off', { timeout: 1500 }); break; } catch {}
}
await page.waitForFunction(() => window.__rackState === 'patched', { timeout: 20000 });
const cables = await page.evaluate(() => window.__rackCables);
console.log(`patched with ${cables} cables (full voice = 7)`);
if (cables !== 7) throw new Error(`expected 7 cables, got ${cables}`);

// Hold a chord on the keyboard piano.
await page.keyboard.down('a');
await page.keyboard.down('d');
await page.keyboard.down('g');
await page.waitForFunction(() => window.__rackNotes === 3, { timeout: 5000 });
console.log('3 notes held');
await page.waitForTimeout(800);
await page.keyboard.up('a');
await page.keyboard.up('d');
await page.keyboard.up('g');
await page.waitForFunction(() => window.__rackNotes === 0, { timeout: 5000 });
console.log('all released');

// Key repeat / stuck note safety: rapid press cycles.
for (let i = 0; i < 5; i++) {
  await page.keyboard.down('h');
  await page.waitForTimeout(60);
  await page.keyboard.up('h');
}
await page.waitForFunction(() => window.__rackNotes === 0, { timeout: 5000 });

const state = await page.evaluate(() => window.__rackState);
if (state !== 'patched') throw new Error(`engine died: ${state}`);
if (errors.length) throw new Error(`page errors: ${errors.join('; ')}`);
console.log('keyboard piano browser test PASSED');
await browser.close();
