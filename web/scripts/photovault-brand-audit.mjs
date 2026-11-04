#!/usr/bin/env node
// PHOTOVAULT: strip (new file).
//
// Lists every string literal, template literal part and JSX text (attribute
// values included) that contains an Ente word, case-insensitively and with
// inflections (`\bente\w*`, minus "enter...", "entend...", "entent..."), in
// the TypeScript sources of the apps Photovault ships. Used for
// PHOTOVAULT-GREP.md. Run from web/: `node scripts/photovault-brand-audit.mjs`.
//
// Import/export module names (`ente-base/...`) are skipped; everything else
// is printed with file:line so each hit can be fixed or explained.

import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import ts from "typescript";

const web = path.join(path.dirname(fileURLToPath(import.meta.url)), "..");
const roots = [
    "apps/photos/src",
    "apps/albums/src",
    "apps/accounts/src",
    "apps/cast/src",
    "apps/embed/src",
    "packages",
];
const skipDirs = new Set(["node_modules", "pkg", "out", ".next", "tests"]);
const wordRE = /\bente(?!r|nd|nt)[\p{L}\d]*/giu;

const files = [];
const walk = (dir) => {
    for (const e of fs.readdirSync(dir, { withFileTypes: true })) {
        if (e.isDirectory()) {
            if (!skipDirs.has(e.name)) walk(path.join(dir, e.name));
        } else if (/\.(ts|tsx)$/.test(e.name) && !/\.d\.ts$/.test(e.name)) {
            files.push(path.join(dir, e.name));
        }
    }
};
for (const r of roots) walk(path.join(web, r));

let count = 0;
for (const file of files.sort()) {
    const text = fs.readFileSync(file, "utf8");
    const sf = ts.createSourceFile(
        file,
        text,
        ts.ScriptTarget.Latest,
        true,
        file.endsWith(".tsx") ? ts.ScriptKind.TSX : ts.ScriptKind.TS,
    );
    const visit = (node) => {
        if (
            ts.isImportDeclaration(node) ||
            ts.isExportDeclaration(node) ||
            (ts.isCallExpression(node) &&
                node.expression.kind == ts.SyntaxKind.ImportKeyword)
        ) {
            return;
        }
        let value;
        if (
            ts.isStringLiteral(node) ||
            ts.isNoSubstitutionTemplateLiteral(node) ||
            ts.isTemplateHead(node) ||
            ts.isTemplateMiddle(node) ||
            ts.isTemplateTail(node) ||
            ts.isJsxText(node)
        ) {
            value = node.text;
        }
        if (value && wordRE.test(value)) {
            wordRE.lastIndex = 0;
            const { line } = sf.getLineAndCharacterOfPosition(node.getStart());
            const shown = value.trim().replace(/\s+/g, " ").slice(0, 140);
            console.log(`${path.relative(web, file)}:${line + 1}: ${shown}`);
            count++;
        }
        wordRE.lastIndex = 0;
        ts.forEachChild(node, visit);
    };
    visit(sf);
}
console.error(`${count} hit(s) in ${files.length} files`);
