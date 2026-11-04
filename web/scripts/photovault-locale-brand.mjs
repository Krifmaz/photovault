#!/usr/bin/env node
// PHOTOVAULT: strip (new file).
//
// Replaces the brand name "Ente" in the values of the shared locale files
// (packages/base/locales/*/translation.json) with "Photovault", then reports
// every value that still contains an Ente word. Re-run it after taking
// upstream's locale files in a merge (see PHOTOVAULT.md "Merging upstream").
//
//   node scripts/photovault-locale-brand.mjs          replace, then check
//   node scripts/photovault-locale-brand.mjs --check  only check (exit 1 on hits)
//
// Keys, URLs and e-mail addresses are untouched; so are "Ente Auth", "Ente
// Locker", "Ente Legacy" and "Ensu" (other Ente products), and the
// title_auth / title_locker values (used only by Ente's Auth and Locker apps).
//
// Inflected forms are language-specific, so they are listed per locale below.
// The check catches any inflected form that is not listed yet
// (`Ente[a-zäöüß...]*`, ignoring "Enter...", "Entend...", "Entent..."), and a
// lowercase "ente" (a real word in Italian, Spanish and Portuguese, so it is
// reported, not replaced).
//
// The files are edited as text, one `"key": "value"` line at a time, so their
// formatting stays exactly as upstream's.

import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const here = path.dirname(fileURLToPath(import.meta.url));
const localesDir = path.join(here, "../packages/base/locales");
const checkOnly = process.argv.includes("--check");

/** Keys whose values name Ente's other apps; left as upstream. */
const skipKeys = new Set(["title_auth", "title_locker"]);

/** Inflected forms, per locale (applied before the generic rules). */
const inflected = {
    "de-DE": [
        [/\bdes Entes\b/g, "von Photovault"],
        [/\bEntes Hilfe\b/g, "Photovault-Hilfe"],
        [/\bEntes\b/g, "Photovaults"],
    ],
    "fi-FI": [
        [/\bEnten\b/g, "Photovaultin"],
        [/\bEntestä(?!\p{L})/gu, "Photovaultista"],
        [/\bEnteen\b/g, "Photovaultiin"],
        [/\bEnteä(?!\p{L})/gu, "Photovaultia"],
    ],
};

/** Generic rules for every locale. */
const generic = [
    [/\bEnte Photos\b/g, "Photovault"],
    // Not followed by a Latin letter (an unlisted inflection is reported by the
    // check instead), but CJK / Hangul / Telugu text may follow directly.
    [
        /\bEnte(?![\p{Script=Latin}])(?!\s+(Auth|Locker|Legacy|Ensu)\b)/gu,
        "Photovault",
    ],
];

const leftoverRE = /\bEnte(?!r|nd|nt)[\p{Ll}]*|\bente\b/u;

const lineRE = /^(\s*"((?:[^"\\]|\\.)*)":\s*")((?:[^"\\]|\\.)*)("\s*,?\s*)$/;

let changed = 0;
const leftovers = [];

for (const locale of fs.readdirSync(localesDir).sort()) {
    const file = path.join(localesDir, locale, "translation.json");
    if (!fs.existsSync(file)) continue;
    const lines = fs.readFileSync(file, "utf8").split("\n");
    let fileChanged = false;
    for (let i = 0; i < lines.length; i++) {
        const m = lineRE.exec(lines[i]);
        if (!m) continue;
        const [, head, key, value, tail] = m;
        if (skipKeys.has(key)) continue;
        let v = value;
        if (!checkOnly) {
            for (const [re, to] of [...(inflected[locale] ?? []), ...generic])
                v = v.replace(re, to);
            if (v != value) {
                lines[i] = head + v + tail;
                fileChanged = true;
                changed++;
            }
        }
        if (leftoverRE.test(v)) leftovers.push(`${locale} ${key}: ${v}`);
    }
    if (fileChanged) fs.writeFileSync(file, lines.join("\n"));
}

if (!checkOnly) console.log(`replaced in ${changed} value(s)`);
if (leftovers.length) {
    console.log("values that still contain an Ente word:");
    for (const l of leftovers) console.log(`  ${l}`);
    process.exit(1);
}
console.log("no Ente words left (outside title_auth/title_locker)");
