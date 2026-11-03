// PHOTOVAULT: branding (new file).
//
// Replaces Ente's duck mascot illustrations in the Photos app with the
// Photovault glyph (assets/photovault_glyph.svg), keeping each file's name and
// pixel size so no layout changes. PNGs get the glyph centred at 60% of the
// shorter side on a transparent background, in a gold gradient that reads on
// both the light and dark Encore themes (accent2 light #8a7a4e -> accent2 dark
// #d6c49a). SVGs are rewritten with the same width/height/viewBox.
//
// Usage (from the repository root):
//   node mobile/apps/photos/scripts/photovault/render_mascot_replacements.js .
// Needs Playwright (npm) and Chrome; set CHROME (default /usr/bin/google-chrome)
// and PLAYWRIGHT (module path) if they are not on the default paths.
const fs = require("fs");
const path = require("path");
const { chromium } = require(process.env.PLAYWRIGHT || "playwright");

const repo = process.argv[2] || ".";
const assets = path.join(repo, "mobile/apps/photos/assets");

// Every raster/SVG asset that showed the duck mascot (all densities).
const PNGS = [
  "albums-widget-static.png",
  "backup_mode_ducky.png",
  "ducky_full_library.png",
  "ducky_garbage_bin_opened.png",
  "ducky_ml.png",
  "ducky_settings.png",
  "ducky_share.png",
  "ducky_smart_feature.png",
  "enable-streaming-static.png",
  "family_plan_illustration.png",
  "feed.png",
  "legacy.png",
  "memories-widget-static.png",
  "on_device.png",
  "on_device_empty.png",
  "people-widget-static.png",
  "received.png",
  "shared.png",
];
const BADGES = [
  "rewind_badges/consistency_champ.png",
  "rewind_badges/globetrotter.png",
  "rewind_badges/minimalist_shooter.png",
  "rewind_badges/people_person.png",
  "rewind_badges/portrait_pro.png",
];
const SVGS = [
  "ducky_cleaning_static.svg",
  "photo_review/all_cleaned_up.svg",
  "rituals/ducky_ritual.svg",
  "rituals/ritual_ducky_share.svg",
];

const glyph = fs.readFileSync(path.join(assets, "photovault_glyph.svg"), "utf8");
const glyphBody = glyph
  .replace(/^[\s\S]*?<svg[^>]*>/, "")
  .replace(/<\/svg>\s*$/, "")
  .replace(/<!--[\s\S]*?-->/g, "")
  .replace(/#000000/g, "url(#pvg)")
  .replace("<defs>", '<defs><linearGradient id="pvg" x1="160" y1="864" x2="864" y2="160" gradientUnits="userSpaceOnUse"><stop offset="0" stop-color="#8a7a4e"/><stop offset="1" stop-color="#d6c49a"/></linearGradient>');

// Glyph (glyph viewBox is 150..874) centred in a w x h box at `scale`.
function glyphSvg(w, h, scale, attrs) {
  const side = Math.min(w, h) * scale;
  const k = side / 724;
  const tx = (w - side) / 2 - 150 * k;
  const ty = (h - side) / 2 - 150 * k;
  return (
    `<svg xmlns="http://www.w3.org/2000/svg" ${attrs}>\n` +
    `  <!-- PHOTOVAULT: branding. Photovault glyph in place of Ente's duck mascot (scripts/photovault/render_mascot_replacements.js). -->\n` +
    `  <g transform="translate(${tx.toFixed(2)} ${ty.toFixed(2)}) scale(${k.toFixed(5)})">${glyphBody}</g>\n</svg>\n`
  );
}

function pngSize(file) {
  const b = fs.readFileSync(file);
  return { w: b.readUInt32BE(16), h: b.readUInt32BE(20) };
}

(async () => {
  const browser = await chromium.launch({
    executablePath: process.env.CHROME || "/usr/bin/google-chrome",
  });
  const page = await browser.newPage();
  const rasters = [];
  for (const rel of [...PNGS, ...BADGES]) {
    for (const dir of ["", "2.0x/", "3.0x/"]) {
      const d = path.dirname(rel) === "." ? "" : path.dirname(rel) + "/";
      const file = path.join(assets, d + dir + path.basename(rel));
      if (fs.existsSync(file)) rasters.push(file);
    }
  }
  for (const file of rasters) {
    const { w, h } = pngSize(file);
    const svg = glyphSvg(w, h, 0.6, `width="${w}" height="${h}" viewBox="0 0 ${w} ${h}"`);
    await page.setViewportSize({ width: w, height: h });
    await page.setContent(
      `<html><body style="margin:0;background:transparent">${svg}</body></html>`,
    );
    await page.locator("svg").screenshot({ path: file, omitBackground: true });
    console.log("png", path.relative(repo, file), w + "x" + h);
  }
  for (const rel of SVGS) {
    const file = path.join(assets, rel);
    const src = fs.readFileSync(file, "utf8");
    const root = src.match(/<svg[^>]*>/)[0];
    const vb = root.match(/viewBox="([^"]+)"/);
    const [, , vw, vh] = vb ? vb[1].split(/[\s,]+/).map(Number) : [0, 0, 0, 0];
    const width = (root.match(/\swidth="([^"]+)"/) || [])[1];
    const height = (root.match(/\sheight="([^"]+)"/) || [])[1];
    const w = vw || parseFloat(width);
    const h = vh || parseFloat(height);
    const attrs =
      (width ? `width="${width}" ` : "") +
      (height ? `height="${height}" ` : "") +
      `viewBox="0 0 ${w} ${h}" fill="none"`;
    fs.writeFileSync(file, glyphSvg(w, h, 0.6, attrs));
    console.log("svg", rel, w + "x" + h);
  }
  await browser.close();
})();
