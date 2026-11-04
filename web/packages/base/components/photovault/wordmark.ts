// PHOTOVAULT: branding (new file).
//
// The Photovault wordmark: the vault/aperture glyph from `branding/glyph.svg`
// followed by the name. The name uses `currentColor` (like Ente's wordmark
// did) so it follows the surrounding text colour; the glyph keeps Encore's
// cream-to-gold gradient.

import { pvDark } from "../utils/photovault/encore-tokens";

/** viewBox of {@link pvWordmarkSVGHTML}, the same height as Ente's (264). */
export const pvWordmarkViewBox = "0 0 1500 264";

const bolts = [
    [807.6, 634.5],
    [634.5, 807.6],
    [389.5, 807.6],
    [216.4, 634.5],
    [216.4, 389.5],
    [389.5, 216.4],
    [634.5, 216.4],
    [807.6, 389.5],
]
    .map(([x, y]) => `<circle cx="${x}" cy="${y}" r="12" fill="#000"/>`)
    .join("");

/**
 * The glyph as SVG markup in a 1024×1024 user space. `id` must be unique per
 * document, since gradient, mask and clip references are global.
 */
const pvGlyphSVGHTML = (id: string) => `
<defs>
  <linearGradient id="${id}-g" x1="160" y1="864" x2="864" y2="160" gradientUnits="userSpaceOnUse">
    <stop offset="0" stop-color="${pvDark.accent2}"/>
    <stop offset="1" stop-color="${pvDark.accent}"/>
  </linearGradient>
  <mask id="${id}-m"><rect width="1024" height="1024" fill="#fff"/>${bolts}</mask>
  <clipPath id="${id}-c"><circle cx="512" cy="512" r="200"/></clipPath>
</defs>
<circle cx="512" cy="512" r="320" fill="none" stroke="url(#${id}-g)" stroke-width="52" mask="url(#${id}-m)"/>
<circle cx="512" cy="512" r="200" fill="none" stroke="url(#${id}-g)" stroke-width="30"/>
<path clip-path="url(#${id}-c)" fill="none" stroke="url(#${id}-g)" stroke-width="26" d="M592.0 465.8 L592.0 812.0 M592.0 558.2 L292.2 731.3 M512.0 604.4 L212.2 431.3 M432.0 558.2 L432.0 212.0 M432.0 465.8 L731.8 292.7 M512.0 419.6 L811.8 592.7"/>`;

/** Inner markup for an `<svg viewBox={pvWordmarkViewBox}>`. */
export const pvWordmarkSVGHTML = (id: string) => `
<svg x="0" y="0" width="264" height="264" viewBox="166 166 692 692">${pvGlyphSVGHTML(id)}</svg>
<text x="316" y="204" textLength="1170" lengthAdjust="spacingAndGlyphs" fill="currentColor"
  font-family="'Inter Variable', Inter, ui-sans-serif, system-ui, sans-serif" font-size="212" font-weight="600">Photovault</text>`;
