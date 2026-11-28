// Runs src/page.html in each engine against `dcspike serve` and prints the result.
// usage: PLAYWRIGHT_CORE=/path/to/playwright-core node browsers.mjs <url> [engines...]
import { createRequire } from 'node:module';
const require = createRequire(import.meta.url);
const pw = require(process.env.PLAYWRIGHT_CORE); // path to a playwright-core whose browsers are installed
const url = process.argv[2];
const engines = process.argv.slice(3).length ? process.argv.slice(3) : ['chromium', 'firefox', 'webkit'];
for (const name of engines) {
  const opts = {};
  if (name === 'chromium') opts.args = ['--disable-features=WebRtcHideLocalIpsWithMdns'];
  if (name === 'firefox') opts.firefoxUserPrefs = { 'media.peerconnection.ice.obfuscate_host_addresses': false };
  let browser;
  try {
    browser = await pw[name].launch(opts);
    const page = await browser.newPage();
    await page.goto(url);
    await page.waitForFunction(() => window.result, null, { timeout: 60000 });
    const r = await page.evaluate(() => window.result);
    console.log(name, JSON.stringify(r));
  } catch (e) { console.log(name, 'ERROR', String(e).slice(0, 300)); }
  finally { await browser?.close(); }
}
