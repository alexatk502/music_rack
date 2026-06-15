import { chromium } from 'playwright-core';
const shell = '/home/atkinsona/.cache/ms-playwright/chromium_headless_shell-1223/chrome-headless-shell-linux64/chrome-headless-shell';
const browser = await chromium.launch({ executablePath: shell, args: ['--no-sandbox', '--autoplay-policy=no-user-gesture-required'] });
const page = await browser.newPage({ viewport: { width: 1280, height: 800 } });
const errors = [];
page.on('pageerror', (e) => errors.push(e.message));
await page.goto('http://127.0.0.1:8123/app.html');
await page.evaluate(() => localStorage.clear());
await page.reload();
await page.waitForFunction(() => window.__rackState === 'off', { timeout: 20000 });
for (const x of [120, 150, 180, 210, 240, 270]) {
  await page.mouse.click(x, 18);
  try { await page.waitForFunction(() => window.__rackState !== 'off', { timeout: 1500 }); break; } catch {}
}
await page.waitForFunction(() => window.__rackState === 'patched', { timeout: 20000 });
const cables0 = await page.evaluate(() => window.__rackCables);
console.log(`patched, ${cables0} cables`);
if (cables0 !== 7) throw new Error(`expected 7 demo cables, got ${cables0}`);

// Detach VCA "in" (VCA at col 58 → panel x≈588, port ≈ (604, 312)).
await page.mouse.move(604, 312);
await page.mouse.down();
await page.mouse.move(700, 600, { steps: 8 });
await page.mouse.up();
await page.waitForFunction(() => window.__rackCables === 6, { timeout: 5000 });
console.log('after detach: 6 cables');

// Reconnect from VCF "out" (col 30 → out port ≈ (402, 312)) to VCA "in".
await page.mouse.move(402, 312);
await page.mouse.down();
await page.mouse.move(604, 312, { steps: 10 });
await page.mouse.up();
await page.waitForFunction(() => window.__rackCables === 7, { timeout: 5000 });
console.log('after reconnect: 7 cables');

const state = await page.evaluate(() => window.__rackState);
if (state !== 'patched') throw new Error(`engine died: ${state}`);
if (errors.length) throw new Error(`page errors: ${errors.join('; ')}`);
console.log('rack UI browser test PASSED');
await browser.close();
