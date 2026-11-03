// Minimal static file server for a Next.js static export, with the same
// fallback rules as upstream's nginx config: $uri, $uri.html, /index.html.
//
//   node static-server.mjs <root-dir> <port>
import { createReadStream, statSync } from "node:fs";
import { createServer } from "node:http";
import path from "node:path";

const [root, portArg] = process.argv.slice(2);
if (!root || !portArg) {
    console.error("usage: static-server.mjs <root> <port>");
    process.exit(2);
}
const rootDir = path.resolve(root);
const port = Number(portArg);

const types = {
    ".html": "text/html; charset=utf-8",
    ".js": "text/javascript; charset=utf-8",
    ".mjs": "text/javascript; charset=utf-8",
    ".css": "text/css; charset=utf-8",
    ".json": "application/json",
    ".wasm": "application/wasm",
    ".svg": "image/svg+xml",
    ".png": "image/png",
    ".jpg": "image/jpeg",
    ".ico": "image/x-icon",
    ".webp": "image/webp",
    ".woff": "font/woff",
    ".woff2": "font/woff2",
    ".txt": "text/plain; charset=utf-8",
    ".webmanifest": "application/manifest+json",
};

const isFile = (p) => {
    try {
        return statSync(p).isFile();
    } catch {
        return false;
    }
};

const resolve = (urlPath) => {
    let decoded;
    try {
        decoded = decodeURIComponent(urlPath);
    } catch {
        return undefined;
    }
    const rel = path.normalize(decoded).replace(/^([/\\])+/, "");
    const abs = path.join(rootDir, rel);
    if (abs !== rootDir && !abs.startsWith(rootDir + path.sep)) return undefined;
    for (const candidate of [abs, abs + ".html", path.join(abs, "index.html")]) {
        if (isFile(candidate)) return candidate;
    }
    return path.join(rootDir, "index.html");
};

createServer((req, res) => {
    const url = new URL(req.url ?? "/", "http://localhost");
    const file = resolve(url.pathname);
    if (!file || !isFile(file)) {
        res.writeHead(404).end("not found");
        return;
    }
    res.writeHead(200, {
        "Content-Type": types[path.extname(file)] ?? "application/octet-stream",
        "Cache-Control": "no-store",
    });
    createReadStream(file).pipe(res);
}).listen(port, "127.0.0.1", () => {
    console.log(`serving ${rootDir} on http://localhost:${port}`);
});
