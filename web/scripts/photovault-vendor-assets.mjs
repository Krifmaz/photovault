#!/usr/bin/env node
// PHOTOVAULT: strip (new file).
//
// Ente's web app loads a few static data files from Ente's CDN
// (assets.ente.com). Photovault loads them from its own origin instead, under
// /static-assets/ (see `staticAssetURL` in packages/base/origins.ts), so the
// browser never contacts Ente. This script copies them into the Photos app's
// public folder before `next build`, so they end up in the static export and
// in the desktop app, which bundles that export (desktop/package.json
// `build-renderer*` runs it; it is Node so it also works on Windows).
//
// Without them the app still works; only these features degrade:
//   - Places (photos grouped by city): location/v2/{cities,urban-centres}.bin
//     (the mobile app also needs location/v2/{countries,disputes}.bin, and
//     reads all four from this server's /static-assets/location/v2)
//   - City names in search:            world_cities.json
//   - Video processing in the browser: ffmpeg-core-9.0.2/ffmpeg-core.{js,wasm}
//     (the desktop app uses its bundled native ffmpeg instead)
//
// Usage: node scripts/photovault-vendor-assets.mjs [SOURCE]
//   SOURCE is a base URL or a local directory with the same layout. The default
//   is $PHOTOVAULT_ASSETS_SOURCE, else Ente's CDN, fetched once at build time
//   (never by users' browsers). PHOTOVAULT_SKIP_ASSETS=1 skips the files (the
//   features above then do not work); otherwise a failed copy fails the build.
//   Downloads use `curl` (honours HTTPS_PROXY; ships with Windows 10+), run
//   without a shell and with fixed arguments.

import { execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const here = path.dirname(fileURLToPath(import.meta.url));
const src =
    process.argv[2] ||
    process.env.PHOTOVAULT_ASSETS_SOURCE ||
    "https://assets.ente.com";

if (process.env.PHOTOVAULT_SKIP_ASSETS == "1") {
    console.warn("PHOTOVAULT_SKIP_ASSETS=1: static data files not vendored");
    process.exit(0);
}
const dest = path.join(here, "../apps/photos/public/static-assets");

const files = [
    "location/v2/cities.bin",
    "location/v2/urban-centres.bin",
    // Not used by the web app, but the mobile app's location index loads all
    // four location files from <server>/static-assets/location/v2.
    "location/v2/countries.bin",
    "location/v2/disputes.bin",
    "world_cities.json",
    "ffmpeg-core-9.0.2/ffmpeg-core.js",
    "ffmpeg-core-9.0.2/ffmpeg-core.wasm",
];

const isURL = /^https?:\/\//.test(src);

for (const f of files) {
    const out = path.join(dest, ...f.split("/"));
    fs.mkdirSync(path.dirname(out), { recursive: true });
    if (isURL) {
        execFileSync(
            "curl",
            [
                "--fail",
                "--silent",
                "--show-error",
                "--location",
                "--output",
                out,
                `${src.replace(/\/+$/, "")}/${f}`,
            ],
            { stdio: "inherit" },
        );
    } else {
        fs.copyFileSync(path.join(src, ...f.split("/")), out);
    }
    console.log(`vendored ${f}`);
}
