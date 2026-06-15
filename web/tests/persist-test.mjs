import { chromium } from 'playwright-core';
const shell = '/home/atkinsona/.cache/ms-playwright/chromium_headless_shell-1223/chrome-headless-shell-linux64/chrome-headless-shell';
const browser = await chromium.launch({ executablePath: shell, args: ['--no-sandbox', '--autoplay-policy=no-user-gesture-required'] });
const ctx = await browser.newContext({ viewport: { width: 1280, height: 800 } });
const page = await ctx.newPage();
const errors = [];
page.on('pageerror', (e) => errors.push(e.message));

const powerOn = async () => {
  for (const x of [120, 150, 180, 210, 240, 270]) {
    await page.mouse.click(x, 18);
    try { await page.waitForFunction(() => window.__rackState !== 'off', { timeout: 1500 }); break; } catch {}
  }
  await page.waitForFunction(() => window.__rackState === 'patched', { timeout: 20000 });
};

await page.goto('http://127.0.0.1:8123/app.html');
await page.evaluate(() => localStorage.clear());
await page.reload();
await page.waitForFunction(() => window.__rackState === 'off', { timeout: 20000 });
await powerOn();
let cables = await page.evaluate(() => window.__rackCables);
console.log(`fresh demo patch: ${cables} cables`);
if (cables !== 7) throw new Error(`expected 7, got ${cables}`);

// Edit topology: detach the VCO v/oct cable. NoteIn col 2 → x≈28..138.
// VCO col 16 → x≈168..278; its "v/oct" input port x≈168+16=184, y≈312.
await page.mouse.move(184, 312);
await page.mouse.down();
await page.mouse.move(700, 600, { steps: 8 });
await page.mouse.up();
await page.waitForFunction(() => window.__rackCables === 6, { timeout: 5000 });
console.log('detached one cable');
// Wait past the autosave debounce.
await page.waitForTimeout(2000);

// Reload: the edited patch must come back from localStorage.
await page.reload();
await page.waitForFunction(() => window.__rackState === 'off', { timeout: 20000 });
cables = await page.evaluate(() => window.__rackCables);
console.log(`after reload: ${cables} cables`);
if (cables !== 6) throw new Error(`persistence failed: expected 6, got ${cables}`);

// Sanity: stored JSON looks right.
const stored = await page.evaluate(() => JSON.parse(localStorage.getItem('music_rack_patch')));
if (stored.version !== 1 || stored.modules.length !== 6) {
  throw new Error(`bad stored patch: ${JSON.stringify(stored).slice(0, 120)}`);
}
console.log(`stored patch: version ${stored.version}, ${stored.modules.length} modules, next_id ${stored.next_id}`);

if (errors.length) throw new Error(`page errors: ${errors.join('; ')}`);
console.log('persistence browser test PASSED');
await browser.close();
