// PHOTOVAULT: branding (new file).
//
// Rasterises branding/icon.svg and branding/glyph.svg into every launcher
// icon, adaptive-icon layer, splash image and in-app logo of the Photos app,
// and writes the "Photovault" wordmark SVGs. Re-run after changing the
// branding or after an upstream merge that adds icon assets.
//
// Usage (from the repository root):
//   python3 mobile/apps/photos/scripts/photovault/wordmark.py \
//     mobile/apps/photos/fonts/Inter-SemiBold.ttf /tmp/wordmark.json <fonttools dir>
//   node mobile/apps/photos/scripts/photovault/render_branding.js . /tmp/wordmark.json
//
// Needs Playwright (npm) and a Chrome/Chromium; set CHROME to its path
// (default /usr/bin/google-chrome). Output sizes follow the existing files.
const fs = require("fs");
const path = require("path");
const { chromium } = require(process.env.PLAYWRIGHT || "playwright");

const repo = process.argv[2];
const word = JSON.parse(fs.readFileSync(process.argv[3], "utf8"));
const app = path.join(repo, "mobile/apps/photos");
const res = path.join(app, "android/app/src/main/res");
const iosAssets = path.join(app, "ios/Runner/Assets.xcassets");

const iconSvg = fs.readFileSync(path.join(repo, "branding/icon.svg"), "utf8");
const glyphSvg = fs.readFileSync(path.join(repo, "branding/glyph.svg"), "utf8");

// Inner markup of an <svg> (everything between the root tags).
function inner(svg) {
  return svg.replace(/^[\s\S]*?<svg[^>]*>/, "").replace(/<\/svg>\s*$/, "");
}

const ICON = inner(iconSvg);
const ICON_SQUARE = ICON.replace('rx="225" ', "");
const GLYPH = inner(glyphSvg);
// Single-colour silhouette of the glyph (for monochrome/notification icons).
const glyphMono = (c) => GLYPH.replace(/url\(#g\)/g, c);
// Grey gradient glyph for iOS "tinted" appearance.
const GLYPH_TINT = GLYPH.replace('stop-color="#d6c49a"', 'stop-color="#9a9a9a"').replace(
  'stop-color="#f0ead6"',
  'stop-color="#ffffff"',
);

// Wrap 1024-unit artwork, scaled by `scale` around the centre, on an
// optional solid background.
function art(body, { scale = 1, bg = null } = {}) {
  const off = (1024 * (1 - scale)) / 2;
  return (
    `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1024 1024" width="1024" height="1024">` +
    (bg ? `<rect width="1024" height="1024" fill="${bg}"/>` : "") +
    `<g transform="translate(${off} ${off}) scale(${scale})">${body}</g></svg>`
  );
}

// Wordmark ("Photovault" in Inter SemiBold) as a standalone SVG.
function wordmarkSvg({ fill = "black", opacity = 1, pad = 0 }) {
  const w = word.xMax - word.xMin + 2 * pad;
  const h = word.yMax - word.yMin + 2 * pad;
  const vb = `${word.xMin - pad} ${word.yMin - pad} ${w} ${h}`;
  return {
    svg:
      `<svg width="${((30 * w) / h).toFixed(1)}" height="30" viewBox="${vb}" fill="none" xmlns="http://www.w3.org/2000/svg">\n` +
      `<!-- PHOTOVAULT: "Photovault" wordmark, Inter SemiBold outlined (replaces Ente's wordmark). -->\n` +
      `<path d="${word.d}" fill="${fill}"${opacity < 1 ? ` fill-opacity="${opacity}"` : ""}/>\n</svg>\n`,
    aspect: w / h,
  };
}

const jobs = [];
const job = (out, svg, w, h = w, opts = {}) => jobs.push({ out, svg, w, h, ...opts });

// ---------- Android ----------
const dens = { mdpi: 1, hdpi: 1.5, xhdpi: 2, xxhdpi: 3, xxxhdpi: 4 };
for (const [d, m] of Object.entries(dens)) {
  // Legacy (pre-26) launcher icons: full rounded tile.
  for (const n of ["ic_launcher", "launcher_icon", "icon_green", "icon_light", "icon_dark", "icon_og"]) {
    job(`${res}/mipmap-${d}/${n}.png`, art(ICON), 48 * m);
  }
  job(`${res}/mipmap-${d}/icon_monochrome.png`, art(glyphMono("#ffffff"), { scale: 1.15 }), 48 * m);
  job(`${app}/android/app/src/dev/res/mipmap-${d}/launcher_icon.png`, art(ICON), 48 * m);
  // Adaptive icon foregrounds (108dp canvas, 66dp safe zone).
  for (const n of ["ic_launcher_foreground", "ic_og_launcher_foreground"]) {
    job(`${res}/drawable-${d}/${n}.png`, art(GLYPH, { scale: 0.82 }), 108 * m);
  }
  job(
    `${res}/drawable-${d}/ic_monochrome_launcher_foreground.png`,
    art(glyphMono("#ffffff"), { scale: 0.82 }),
    108 * m,
  );
  // Splash: pre-12 launch_background (light and dark share it) and
  // Android 12 splash icon (light: tile; dark: bare glyph).
  job(`${res}/drawable-${d}/splash.png`, art(ICON, { scale: 0.5 }), 288 * m);
  job(`${res}/drawable-${d}/android12splash.png`, art(ICON, { scale: 0.46 }), 288 * m);
  job(`${res}/drawable-night-${d}/android12splash.png`, art(GLYPH, { scale: 0.92 }), 288 * m);
}
job(`${res}/drawable/notification_icon.png`, art(glyphMono("#ffffff"), { scale: 1.2 }), 96);
job(`${res}/drawable/background.png`, art("", { bg: "#f6f6fb" }), 1);
job(`${res}/drawable-v21/background.png`, art("", { bg: "#f6f6fb" }), 1);
job(`${res}/drawable-night/background.png`, art("", { bg: "#161617" }), 1);
job(`${res}/drawable-night-v21/background.png`, art("", { bg: "#161617" }), 1);
job(`${app}/android/app/src/main/play/listings/en-US/graphics/icon/icon.png`, art(ICON_SQUARE), 512, 512, {
  opaque: true,
});

// ---------- iOS ----------
for (const set of ["IconGreen", "IconLight", "IconDark", "IconOG"]) {
  const dir = `${iosAssets}/${set}.appiconset`;
  job(`${dir}/${set}Any.png`, art(ICON_SQUARE), 1024, 1024, { opaque: true });
  job(`${dir}/${set}Dark.png`, art(GLYPH), 1024);
  job(`${dir}/${set}Tinted.png`, art(GLYPH_TINT, { bg: "#000000" }), 1024, 1024, { opaque: true });
}
for (const [suffix, m] of [["", 1], ["@2x", 2], ["@3x", 3]]) {
  job(`${iosAssets}/LaunchImage.imageset/LaunchImage${suffix}.png`, art(ICON, { scale: 0.6 }), 288 * m);
}
job(`${iosAssets}/LaunchBackground.imageset/background.png`, art("", { bg: "#f6f6fb" }), 1);
job(`${iosAssets}/LaunchBackground.imageset/darkbackground.png`, art("", { bg: "#161617" }), 1);

// ---------- In-app assets ----------
const li = `${app}/assets/launcher_icon`;
const sizeOf = (p) => {
  const b = fs.readFileSync(p);
  return [b.readUInt32BE(16), b.readUInt32BE(20)];
};
for (const n of ["icon-green", "icon-light", "icon-dark", "icon-og", "icon-dev"]) {
  const [w, h] = sizeOf(`${li}/${n}.png`);
  job(`${li}/${n}.png`, art(ICON), w, h);
}
for (const n of ["icon-foreground", "icon-og-foreground"]) {
  const [w, h] = sizeOf(`${li}/${n}.png`);
  job(`${li}/${n}.png`, art(GLYPH, { scale: 0.82 }), w, h);
}
{
  const [w, h] = sizeOf(`${li}/icon-monochrome-foreground.png`);
  job(`${li}/icon-monochrome-foreground.png`, art(glyphMono("#ffffff"), { scale: 0.82 }), w, h);
}
{
  const [w, h] = sizeOf(`${app}/assets/qr_logo.png`);
  job(`${app}/assets/qr_logo.png`, art(ICON), w, h);
}
// Share-export watermark (was white "ente" + green ".io"): cream wordmark.
{
  // The share export draws this into a fixed 116:27 box, so keep that
  // aspect (letterboxed wordmark) at 4x resolution.
  const ww = word.xMax - word.xMin;
  const hh = ww * (27 / 116);
  const py = (hh - (word.yMax - word.yMin)) / 2;
  const svg =
    `<svg width="464" height="108" viewBox="${word.xMin} ${word.yMin - py} ${ww} ${hh}" xmlns="http://www.w3.org/2000/svg">` +
    `<path d="${word.d}" fill="#f0ead6"/></svg>`;
  job(`${app}/assets/ente_io_green.png`, svg, 464, 108, { raw: true });
}
// Vector wordmarks.
fs.writeFileSync(`${app}/assets/ente-branding.svg`, wordmarkSvg({ fill: "black" }).svg);
fs.writeFileSync(`${app}/assets/ente_io_green_black.svg`, wordmarkSvg({ fill: "black", opacity: 0.75 }).svg);

(async () => {
  const browser = await chromium.launch({
    executablePath: process.env.CHROME || "/usr/bin/google-chrome",
  });
  const page = await browser.newPage({ deviceScaleFactor: 1 });
  for (const j of jobs) {
    if (!fs.existsSync(path.dirname(j.out))) throw new Error("missing dir " + j.out);
    await page.setViewportSize({ width: j.w, height: j.h });
    const svg = j.raw
      ? j.svg
      : j.svg.replace('width="1024" height="1024"', `width="${j.w}" height="${j.h}"`);
    await page.setContent(
      `<html><body style="margin:0;background:transparent">${svg}</body></html>`,
    );
    await page.screenshot({
      path: j.out + ".tmp.png",
      omitBackground: !j.opaque,
      clip: { x: 0, y: 0, width: j.w, height: j.h },
    });
    fs.renameSync(j.out + ".tmp.png", j.out);
  }
  await browser.close();
  console.log("rendered", jobs.length);
})();
