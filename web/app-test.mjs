// Headless browser test for the egui app (run from a dir with playwright-core
// installed): boots the app, powers on the engine via the toolbar button,
// drags the PITCH knob, and verifies the engine stays alive with no page
// errors. Usage: node app-test.mjs [base-url]
import { chromium } from 'playwright-core';

const base = process.argv[2] ?? 'http://127.0.0.1:8123';
const shell =
  process.env.CHROME_SHELL ??
  '/home/atkinsona/.cache/ms-playwright/chromium_headless_shell-1223/chrome-headless-shell-linux64/chrome-headless-shell';

const errors = [];
const browser = await chromium.launch({
  executablePath: shell,
  args: ['--no-sandbox', '--autoplay-policy=no-user-gesture-required'],
});
const page = await browser.newPage({ viewport: { width: 1024, height: 768 } });
page.on('pageerror', (e) => errors.push(`pageerror: ${e.message}`));
page.on('console', (m) => {
  if (m.type() === 'error') errors.push(`console: ${m.text()}`);
});

await page.goto(`${base}/app.html`);
await page.waitForFunction(() => window.__rackState === 'off', { timeout: 20000 });
console.log('app booted');

// The power button sits in the toolbar; scan plausible positions since egui
// layouts shift with font metrics.
for (const x of [120, 150, 180, 210, 240, 270]) {
  await page.mouse.click(x, 18);
  try {
    await page.waitForFunction(() => window.__rackState !== 'off', { timeout: 1500 });
    break;
  } catch {
    /* try next position */
  }
}
// 'patched' = engine running AND the demo patch plan has been shipped.
await page.waitForFunction(() => window.__rackState === 'patched', { timeout: 20000 });
console.log('engine running, patch loaded');

// Drag the PITCH knob (first knob in the central panel) up and down.
await page.mouse.move(38, 75);
await page.mouse.down();
await page.mouse.move(38, 25, { steps: 10 });
await page.mouse.move(38, 110, { steps: 10 });
await page.mouse.up();
await page.waitForTimeout(1000);

const state = await page.evaluate(() => window.__rackState);
if (state !== 'patched') throw new Error(`engine died after knob drag: ${state}`);
if (errors.length) throw new Error(`page errors:\n${errors.join('\n')}`);

console.log('app browser test PASSED');
await browser.close();
