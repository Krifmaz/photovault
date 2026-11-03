// Playwright driver for the upstream Ente Photos web app. Started by run.sh;
// can also be run directly against already running services:
//
//   E2E_WEB_URL=http://localhost:13000 E2E_ALBUMS_URL=http://localhost:13002 \
//   E2E_API_URL=http://localhost:18080 node tests/e2e/runner.mjs [test ...]
//
// Every feature is its own test case. Tests share two accounts; a test whose
// dependency failed is reported as "blocked" rather than run. Results go to
// $E2E_ARTIFACTS/results.json; each failing test gets a directory with
// screenshots, the visible text and controls of every open page, console
// output and failed API requests.

import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { createRequire } from "node:module";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { tests } from "./tests.mjs";

const require = createRequire(import.meta.url);
const { chromium } = require("playwright");

const here = path.dirname(fileURLToPath(import.meta.url));
const env = (k, d) => process.env[k] || d;
const config = {
    web: env("E2E_WEB_URL", "http://localhost:13000"),
    albums: env("E2E_ALBUMS_URL", "http://localhost:13002"),
    api: env("E2E_API_URL", "http://localhost:18080"),
    artifacts: env("E2E_ARTIFACTS", path.join(here, "artifacts")),
    tmp: env("E2E_TMP", path.join(here, "artifacts", "tmp")),
    ott: env("E2E_OTT", "123456"),
    testTimeoutMs: Number(env("E2E_TEST_TIMEOUT_MS", "180000")),
};
mkdirSync(config.artifacts, { recursive: true });
mkdirSync(config.tmp, { recursive: true });

/** Generate small, valid test media with ffmpeg (fixed arguments only). */
const makeMedia = () => {
    const dir = path.join(config.tmp, "media");
    mkdirSync(dir, { recursive: true });
    const ff = (args) => execFileSync("ffmpeg", ["-loglevel", "error", "-y", ...args]);
    const files = {
        photo1: path.join(dir, "e2e-photo-1.jpg"),
        photo2: path.join(dir, "e2e-photo-2.jpg"),
        video1: path.join(dir, "e2e-video-1.mp4"),
    };
    ff(["-f", "lavfi", "-i", "testsrc=size=640x480:rate=1", "-frames:v", "1", "-q:v", "3", files.photo1]);
    ff(["-f", "lavfi", "-i", "testsrc2=size=640x480:rate=1", "-frames:v", "1", "-q:v", "3", files.photo2]);
    ff([
        "-f", "lavfi", "-i", "testsrc=size=320x240:rate=25", "-t", "2",
        "-c:v", "libx264", "-pix_fmt", "yuv420p", "-movflags", "+faststart", files.video1,
    ]);
    const sha = {};
    for (const [k, f] of Object.entries(files)) {
        sha[k] = createHash("sha256").update(readFileSync(f)).digest("hex");
    }
    return { files, sha };
};

const safeName = (s) => s.replace(/[^A-Za-z0-9_.-]/g, "_");

/** Per-page diagnostics, collected for failure reports. */
const instrument = (page, label, log) => {
    page.on("console", (m) => {
        if (["error", "warning"].includes(m.type()))
            log.push(`[${label}] console.${m.type()}: ${m.text().slice(0, 500)}`);
    });
    page.on("pageerror", (e) => log.push(`[${label}] pageerror: ${String(e).slice(0, 500)}`));
    page.on("response", (r) => {
        const u = r.url();
        if (r.status() >= 400 && !u.startsWith(config.web) && !u.startsWith(config.albums))
            log.push(`[${label}] HTTP ${r.status()} ${r.request().method()} ${u}`);
    });
    page.on("requestfailed", (r) => {
        const u = r.url();
        if (!u.startsWith("data:"))
            log.push(`[${label}] request failed (${r.failure()?.errorText}) ${r.request?.().method?.() ?? r.method()} ${u}`);
    });
};

const describePage = async (page) => {
    try {
        const text = (await page.locator("body").innerText({ timeout: 3000 })).slice(0, 6000);
        const controls = await page.$$eval(
            "input,button,a,[role=button],[role=checkbox],[role=menuitem],[role=tab],[role=option]",
            (es) =>
                es
                    .filter((e) => e.offsetParent !== null)
                    .map((e) =>
                        [
                            e.tagName.toLowerCase(),
                            e.getAttribute("type") ? `type=${e.getAttribute("type")}` : "",
                            e.getAttribute("name") ? `name=${e.getAttribute("name")}` : "",
                            e.getAttribute("aria-label") ? `aria=${e.getAttribute("aria-label")}` : "",
                            (e.innerText || "").trim().replace(/\s+/g, " ").slice(0, 60),
                        ]
                            .filter(Boolean)
                            .join(" "),
                    ),
        );
        return `URL: ${page.url()}\n\n--- text ---\n${text}\n\n--- visible controls ---\n${controls.join("\n")}\n`;
    } catch (e) {
        return `URL: ${page.url()}\n(could not describe page: ${e})\n`;
    }
};

const main = async () => {
    const only = process.argv.slice(2);
    const unknown = only.filter((n) => !tests.some((t) => t.name === n));
    if (unknown.length) {
        console.error(`unknown tests: ${unknown.join(", ")}\nknown: ${tests.map((t) => t.name).join(", ")}`);
        process.exit(2);
    }
    const media = makeMedia();
    const headless = process.env.E2E_HEADED !== "1";
    const browser = await chromium.launch({ headless });
    const log = [];
    const pages = new Map(); // label -> page, for failure dumps
    const ctx = {
        config,
        media,
        browser,
        state: {},
        log,
        /** New browser context + page, instrumented and registered under `label`. */
        newPage: async (label, opts = {}) => {
            const context = await browser.newContext({
                acceptDownloads: true,
                viewport: { width: 1400, height: 900 },
                permissions: ["clipboard-read", "clipboard-write"],
                ...opts,
            });
            const page = await context.newPage();
            page.setDefaultTimeout(30000);
            instrument(page, label, log);
            pages.set(label, page);
            return page;
        },
        note: (msg) => {
            log.push(`[note] ${msg}`);
            console.log(`    ${msg}`);
        },
    };

    const results = [];
    const status = new Map();
    const selected = only.length ? tests.filter((t) => only.includes(t.name)) : tests;
    for (const t of tests) {
        if (!selected.includes(t)) continue;
        if (t.skip) {
            status.set(t.name, "skip");
            results.push({ name: t.name, status: "skip", reason: t.skip, ms: 0 });
            console.log(`SKIP    ${t.name} (${t.skip})`);
            continue;
        }
        const blockedBy = (t.deps ?? []).filter((d) => status.has(d) && status.get(d) !== "pass");
        const missing = (t.deps ?? []).filter((d) => !status.has(d));
        if (blockedBy.length || (only.length === 0 && missing.length)) {
            const why = blockedBy.length ? `dependency failed: ${blockedBy.join(", ")}` : `dependency not run: ${missing.join(", ")}`;
            status.set(t.name, "blocked");
            results.push({ name: t.name, status: "blocked", reason: why, ms: 0 });
            console.log(`BLOCKED ${t.name} (${why})`);
            continue;
        }
        const started = Date.now();
        const logStart = log.length;
        let timer;
        try {
            await Promise.race([
                t.run(ctx),
                new Promise((_, rej) => {
                    timer = setTimeout(() => rej(new Error(`test timed out after ${config.testTimeoutMs} ms`)), config.testTimeoutMs);
                }),
            ]);
            status.set(t.name, "pass");
            results.push({ name: t.name, status: "pass", ms: Date.now() - started });
            console.log(`PASS    ${t.name} (${Date.now() - started} ms)`);
        } catch (e) {
            status.set(t.name, "fail");
            const err = String(e?.stack ?? e).slice(0, 4000);
            const dir = path.join(config.artifacts, safeName(t.name));
            mkdirSync(dir, { recursive: true });
            for (const [label, page] of pages) {
                if (page.isClosed()) continue;
                await page.screenshot({ path: path.join(dir, `${safeName(label)}.png`), fullPage: false }).catch(() => {});
                writeFileSync(path.join(dir, `${safeName(label)}.txt`), await describePage(page));
            }
            writeFileSync(path.join(dir, "error.txt"), err + "\n");
            writeFileSync(path.join(dir, "log.txt"), log.slice(logStart).join("\n") + "\n");
            const apiErrors = log.slice(logStart).filter((l) => l.includes(" HTTP ")).slice(0, 8);
            results.push({ name: t.name, status: "fail", error: err.split("\n")[0], apiErrors, ms: Date.now() - started });
            console.log(`FAIL    ${t.name}: ${err.split("\n")[0]}`);
            for (const l of apiErrors) console.log(`          ${l}`);
        } finally {
            clearTimeout(timer);
        }
    }
    await browser.close();
    writeFileSync(path.join(config.artifacts, "log.txt"), log.join("\n") + "\n");
    const count = (s) => results.filter((r) => r.status === s).length;
    const summary = { pass: count("pass"), fail: count("fail"), blocked: count("blocked"), skip: count("skip"), total: results.length };
    writeFileSync(path.join(config.artifacts, "results.json"), JSON.stringify({ config, summary, results }, null, 2) + "\n");
    console.log(`\n${summary.pass} passed, ${summary.fail} failed, ${summary.blocked} blocked, ${summary.skip} skipped (of ${summary.total})`);
    process.exit(summary.fail || summary.blocked ? 1 : 0);
};

await main();
