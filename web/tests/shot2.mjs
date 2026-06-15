import { chromium } from 'playwright-core';
const shell = '/home/atkinsona/.cache/ms-playwright/chromium_headless_shell-1223/chrome-headless-shell-linux64/chrome-headless-shell';
const browser = await chromium.launch({ executablePath: shell, args: ['--no-sandbox', '--autoplay-policy=no-user-gesture-required'] });
const page = await browser.newPage({ viewport: { width: 1280, height: 800 } });
page.on('crash', () => console.log('PAGE CRASHED'));
await page.goto('http://127.0.0.1:8123/app.html');
await page.waitForFunction(() => window.__rackState === 'off', { timeout: 30000 });
await page.waitForTimeout(2000);
// Screenshot the powered-off rack first (UI is visible regardless).
await page.screenshot({ path: 'rack-off.png' });
console.log('off screenshot taken');
for (const x of [120, 150, 180, 210, 240, 270]) {
  await page.mouse.click(x, 18);
  await page.waitForTimeout(800);
  const s = await page.evaluate(() => window.__rackState).catch(() => 'crashed');
  if (s !== 'off') { console.log('state:', s); break; }
}
await page.waitForFunction(() => window.__rackState === 'patched', { timeout: 30000 });
await page.waitForTimeout(1500);
await page.screenshot({ path: 'rack-on.png' });
console.log('on screenshot taken');
await browser.close();
