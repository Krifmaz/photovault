// Drives the upstream Electron desktop app (run under xvfb by desktop-smoke.sh).
//   node desktop-smoke.mjs <path-to-ente-executable>
import { mkdirSync, writeFileSync } from "node:fs";
import { createRequire } from "node:module";
import path from "node:path";

const require = createRequire(import.meta.url);
const { _electron } = require("playwright");

const [exe] = process.argv.slice(2);
const api = process.env.E2E_API_URL ?? "http://localhost:18080";
const artifacts = process.env.E2E_ARTIFACTS ?? ".";
mkdirSync(artifacts, { recursive: true });

const app = await _electron.launch({
    executablePath: exe,
    args: ["--no-sandbox"],
    env: { ...process.env, ELECTRON_DISABLE_SANDBOX: "1" },
});
let page;
try {
    page = await app.firstWindow();
    page.setDefaultTimeout(60000);
    const background = page.getByText("Safe home for your photos").first();
    await background.waitFor();
    // Seven taps on the background open "Developer settings" (custom server).
    for (let i = 0; i < 7; i++) {
        await background.click();
        await page.waitForTimeout(150);
    }
    await page.getByText("Developer settings").waitFor();
    await page.locator("input[name=apiOrigin]").fill(api);
    await page.getByRole("button", { name: "Save", exact: true }).click();
    await page.getByText("Developer settings").waitFor({ state: "hidden", timeout: 15000 });
    await page.getByText(new URL(api).host).first().waitFor();
    // The login form now sends requests to our server.
    const seen = new Promise((resolve) =>
        page.on("request", (r) => r.url().startsWith(api) && resolve(r.url())),
    );
    await page.locator("input[name=email]").fill("desktop-smoke@example.com");
    await page.getByRole("button", { name: "Login", exact: true }).first().click();
    const url = await Promise.race([seen, page.waitForTimeout(30000).then(() => undefined)]);
    if (!url) throw new Error(`desktop app sent no request to ${api}`);
    console.log(`PASS desktop smoke: app accepted ${api} and called ${url}`);
    await page.screenshot({ path: path.join(artifacts, "desktop-ok.png") });
} catch (e) {
    console.error(`FAIL desktop smoke: ${e}`);
    if (page) {
        await page.screenshot({ path: path.join(artifacts, "desktop-fail.png") }).catch(() => {});
        writeFileSync(path.join(artifacts, "desktop-fail.txt"), await page.locator("body").innerText().catch(() => ""));
    }
    process.exitCode = 1;
} finally {
    await app.close();
}
