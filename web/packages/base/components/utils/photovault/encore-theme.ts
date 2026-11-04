// PHOTOVAULT: theme (new file).
//
// Maps the Encore tokens onto the palette shape that Ente's `theme.ts` uses,
// so that `theme.ts` only needs a few hooks and stays easy to merge with
// upstream. All values come from `./encore-tokens`.

import type { Components } from "@mui/material/styles";
import {
    pvAlpha,
    pvDark,
    pvLight,
    pvRadius,
    pvVar,
    type PVScheme,
} from "./encore-tokens";

/** An Ente palette scheme (the shape of `_colors.light` in `theme.ts`). */
const scheme = (s: PVScheme, isDark: boolean) => {
    const a = (alpha: number, light: number) =>
        pvAlpha(s.text, isDark ? alpha : light);
    return {
        background: {
            default: s.bg,
            paper: s.panel,
            paper2: s.panel2,
            searchInput: isDark ? s.panel : s.panel2,
        },
        backdrop: {
            base: pvAlpha(s.bg, isDark ? 0.9 : 0.92),
            muted: pvAlpha(s.bg, isDark ? 0.65 : 0.75),
            faint: pvAlpha(s.bg, isDark ? 0.2 : 0.3),
        },
        text: { base: s.text, muted: s.muted, faint: s.faint },
        fill: {
            base: s.text,
            muted: a(0.16, 0.12),
            faint: a(0.12, 0.04),
            faintHover: a(0.16, 0.08),
            fainter: a(0.05, 0.02),
        },
        secondary: { main: s.panel2, hover: s.border },
        stroke: {
            base: s.text,
            muted: a(0.24, 0.24),
            faint: s.border,
            fainter: a(0.12, 0.06),
        },
        boxShadow: isDark
            ? {
                  paper: "0px 2px 12px rgba(0 0 0 / 0.75)",
                  menu: "0px 0px 6px rgba(0 0 0 / 0.50), 0px 3px 6px rgba(0 0 0 / 0.25)",
              }
            : {
                  paper: "0px 0px 10px rgba(0 0 0 / 0.12)",
                  menu: "0px 0px 6px rgba(0 0 0 / 0.10), 0px 3px 6px rgba(0 0 0 / 0.08)",
              },
    };
};

const encoreLight = scheme(pvLight, false);
const encoreDark = scheme(pvDark, true);

interface EnteColors {
    fixed: Record<string, unknown>;
}

/**
 * Replace the colours returned by `theme.ts`'s `getColors` with Encore's.
 * Accent and primary are set per scheme in {@link encoreColorSchemes}.
 */
export const encoreColors = <T extends EnteColors>(colors: T): T => ({
    ...colors,
    fixed: {
        ...colors.fixed,
        success: pvDark.ok,
        golden: pvDark.warn,
        danger: { dark: "#dc2626", main: pvDark.err, light: "#f87171" },
        switchOn: pvDark.accent2,
        dark: {
            background: encoreDark.background,
            text: encoreDark.text,
            divider: encoreDark.stroke.faint,
        },
    },
    light: encoreLight,
    dark: encoreDark,
});

/** Palette entries that differ per scheme in Encore. */
const schemePalette = (s: PVScheme) => ({
    // Ente's "primary" buttons are its neutral (black/white) buttons.
    primary: { main: s.text, contrastText: s.bg2 },
    accent: {
        main: s.accent,
        dark: s.accent2,
        light: s.accent2,
        contrastText: s.onAccent,
    },
    success: { main: s.ok },
    warning: { main: s.warn },
    info: { main: s.info },
});

interface ColorSchemes {
    light: { palette: object };
    dark: { palette: object };
}

/** Apply the per-scheme Encore accent/primary/status colours. */
export const encoreColorSchemes = <T extends ColorSchemes>(cs: T): T => ({
    ...cs,
    light: {
        ...cs.light,
        palette: { ...cs.light.palette, ...schemePalette(pvLight) },
    },
    dark: {
        ...cs.dark,
        palette: { ...cs.dark.palette, ...schemePalette(pvDark) },
    },
});

type Overrides = Record<string, unknown>;

const root = (components: Components, name: keyof Components): Overrides => {
    const c = components[name] as { styleOverrides?: Overrides } | undefined;
    return (c?.styleOverrides?.root ?? {}) as Overrides;
};

/**
 * Encore's component look on top of Ente's overrides: rounded cards (14px)
 * and controls (10px), and cream→gold gradient accent buttons. Only visual
 * properties change; layout and behaviour stay as upstream.
 */
export const encoreComponents = (components: Components): Components => ({
    ...components,
    MuiCssBaseline: {
        ...components.MuiCssBaseline,
        styleOverrides: {
            ...(components.MuiCssBaseline?.styleOverrides as Overrides),
            // The wordmark glyph's gradient follows the scheme's accent
            // (`photovault/wordmark.ts`); light mode gets the charcoal accent.
            ":root": {
                "--pv-glyph-from": pvVar.accent2,
                "--pv-glyph-to": pvVar.accent,
            },
        },
    },
    MuiPaper: {
        ...components.MuiPaper,
        styleOverrides: {
            ...components.MuiPaper?.styleOverrides,
            rounded: { borderRadius: `${pvRadius.card}px` },
        },
    },
    MuiButton: {
        ...components.MuiButton,
        styleOverrides: {
            ...components.MuiButton?.styleOverrides,
            root: {
                ...root(components, "MuiButton"),
                borderRadius: `${pvRadius.control}px`,
                variants: [
                    {
                        props: { variant: "contained", color: "accent" },
                        style: {
                            // Follows the active scheme via MUI's CSS variables.
                            backgroundImage: pvVar.gradient,
                            "&:hover": { filter: "brightness(1.08)" },
                            "&.Mui-disabled": { backgroundImage: "none" },
                        },
                    },
                ],
            },
        },
    },
    MuiInputBase: {
        ...components.MuiInputBase,
        styleOverrides: {
            ...components.MuiInputBase?.styleOverrides,
            formControl: {
                ...(components.MuiInputBase?.styleOverrides
                    ?.formControl as Overrides),
                borderRadius: `${pvRadius.control}px`,
            },
        },
    },
    MuiSnackbar: {
        ...components.MuiSnackbar,
        styleOverrides: { root: { borderRadius: `${pvRadius.control}px` } },
    },
});
