// Dev helper: screenshots of the real UI (real backend via dev_bridge).
//   node tests/ui/shot.mjs <out-dir> [pdf] [scheme]
import { chromium } from "playwright";
import { spawn } from "node:child_process";
import path from "node:path";
import { fileURLToPath } from "node:url";
const APP = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..", "..");
const [out, pdf, scheme = "light-grey"] = process.argv.slice(2);
const PORT = 1700 + Math.floor(Math.random() * 200);
const bridge = spawn(path.join(APP, "src-tauri/target/release/examples/dev_bridge"), [String(PORT), path.join(APP, "dist")]);
await new Promise((r) => bridge.stdout.once("data", r));
const browser = await chromium.launch();
const page = await browser.newPage({ viewport: { width: 1226, height: 860 } });
await page.addInitScript(() => {
  window.__dialog = [];
  window.__TAURI_INTERNALS__ = {
    async invoke(cmd, args) {
      if (cmd.startsWith("plugin:dialog|")) return window.__dialog.shift() ?? null;
      const j = await (await fetch(`/ipc/${cmd}`, { method: "POST", body: JSON.stringify(args ?? {}) })).json();
      if ("err" in j) throw j.err;
      return j.ok;
    },
    transformCallback: () => 0,
    metadata: { currentWindow: { label: "main" }, currentWebview: { windowLabel: "main", label: "main" } },
  };
});
await page.goto(`http://127.0.0.1:${PORT}`);
await page.evaluate((t) => window.__stitchup.theme(t), scheme);
await page.screenshot({ path: path.join(out, `empty-${scheme}.png`) });
if (pdf) {
  await page.evaluate((p) => window.__dialog.push(p), pdf);
  await page.evaluate(() => window.__stitchup.run("open"));
  await page.waitForSelector(".page-view canvas");
  await page.waitForTimeout(1200);
  await page.screenshot({ path: path.join(out, `doc-${scheme}.png`) });
}
await browser.close();
bridge.kill();
