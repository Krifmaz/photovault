// PHOTOVAULT: theme tokens (new file).
//
// The single place the web apps read the Encore look from. The values mirror
// `branding/encore-tokens.json` at the repository root; change them there
// first and then here. Everything else in the web apps references these
// exports (or the MUI CSS variables the theme derives from them) instead of
// hard-coding colours.

import type { AppName } from "ente-base/app";

/** Display name of the product. */
export const pvBrandName = "Photovault";

/** Credit line required by the licence notice; shown in the apps. */
export const pvCredit = "Based on Ente (ente.com)";

/** Link target for {@link pvCredit}. */
export const pvCreditURL = "https://ente.com";

/** Font stacks. Inter is bundled via `@fontsource-variable/inter`. */
export const pvFont = {
    family: '"Inter Variable", Inter, ui-sans-serif, system-ui, -apple-system, "Segoe UI", Roboto, sans-serif',
    mono: "ui-monospace, SFMono-Regular, Menlo, monospace",
};

/** Corner radii in px. */
export const pvRadius = { card: 14, control: 10, small: 8, pill: 99, hero: 20 };

/** One colour scheme of the Encore palette. */
export interface PVScheme {
    bg: string;
    bg2: string;
    panel: string;
    panel2: string;
    border: string;
    text: string;
    muted: string;
    faint: string;
    accent: string;
    accent2: string;
    onAccent: string;
    ok: string;
    warn: string;
    err: string;
    info: string;
}

/** Dark scheme (Encore's default). */
export const pvDark: PVScheme = {
    bg: "#161617",
    bg2: "#1c1c1e",
    panel: "#222224",
    panel2: "#2a2a2d",
    border: "#353538",
    text: "#f0ead6",
    muted: "#aaa493",
    faint: "#77725f",
    accent: "#f0ead6",
    accent2: "#d6c49a",
    onAccent: "#16161f",
    ok: "#22c55e",
    warn: "#f59e0b",
    err: "#ef4444",
    info: "#38bdf8",
};

/** Light scheme (charcoal accent with a gold highlight). */
export const pvLight: PVScheme = {
    bg: "#f6f6fb",
    bg2: "#ffffff",
    panel: "#ffffff",
    panel2: "#f0f0f7",
    border: "#e2e2ee",
    text: "#16161f",
    muted: "#5d5d75",
    faint: "#8d8da5",
    accent: "#2a2a2d",
    accent2: "#8a7a4e",
    onAccent: "#f0ead6",
    ok: "#22c55e",
    warn: "#f59e0b",
    err: "#ef4444",
    info: "#38bdf8",
};

/** Primary-button gradient for a scheme. */
export const pvGradient = (s: PVScheme) =>
    `linear-gradient(135deg, ${s.accent}, ${s.accent2})`;

/** `#rrggbb` + alpha (0..1) → `rgba(r g b / a)`. */
export const pvAlpha = (hex: string, alpha: number) => {
    const n = parseInt(hex.slice(1), 16);
    return `rgba(${(n >> 16) & 255} ${(n >> 8) & 255} ${n & 255} / ${alpha})`;
};

/**
 * Accent to use for hard-coded brand highlights outside the MUI theme (icons,
 * badges, illustrations) that need a single value in both schemes: Encore's
 * gold, which reads on both the charcoal and the light backgrounds.
 */
export const pvGold = pvDark.accent2;

/** Hover/pressed state of {@link pvGold} (the cream end of the gradient). */
export const pvGoldHover = pvDark.accent;

/** Text and icon colour on a {@link pvGold} background. */
export const pvOnGold = pvDark.onAccent;

/**
 * CSS references to the theme's accent, for `sx`/styled values that should
 * follow the active colour scheme.
 */
export const pvVar = {
    accent: "var(--mui-palette-accent-main)",
    accent2: "var(--mui-palette-accent-dark)",
    onAccent: "var(--mui-palette-accent-contrastText)",
    gradient:
        "linear-gradient(135deg, var(--mui-palette-accent-main), var(--mui-palette-accent-dark))",
};

/**
 * The apps that are part of Photovault and use the Encore theme and branding.
 * Ente Auth, Locker, Ensu, Legacy and friends keep their upstream look.
 */
export const isPhotovaultApp = (appName: AppName) =>
    appName == "photos" ||
    appName == "accounts" ||
    appName == "albums" ||
    appName == "cast" ||
    appName == "embed" ||
    appName == "photobook";
