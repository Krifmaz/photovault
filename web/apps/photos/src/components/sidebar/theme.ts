import type { CSSObject, Theme } from "@mui/material/styles";
// PHOTOVAULT: theme
import {
    pvDark as D,
    pvLight as L,
} from "ente-base/components/utils/photovault/encore-tokens";

// PHOTOVAULT: theme - "#rrggbb" to MUI's "r g b" channel format.
const channel = (hex: string) => {
    const n = parseInt(hex.slice(1), 16);
    return `${(n >> 16) & 255} ${(n >> 8) & 255} ${n & 255}`;
};

const inter =
    '"Inter Variable", Inter, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif';

const sidebarHeadingFont = '"Outfit Variable", Outfit, sans-serif';

// Keep the settings palette local, including drawers and portalled menus.
export const sidebarStyles = (theme: Theme): CSSObject => {
    return {
        // PHOTOVAULT: theme - Encore colours (from the tokens) instead of
        // Ente's greys and green.
        "--mui-palette-background-default": L.panel2,
        "--mui-palette-background-paper": L.panel,
        "--mui-palette-background-paper2": L.panel,
        "--mui-palette-text-muted": L.muted,
        "--mui-palette-text-secondary": L.muted,
        "--mui-palette-text-faint": L.faint,
        "--mui-palette-fill-faint": L.panel2,
        "--mui-palette-fill-fainter": L.panel2,
        "--mui-palette-fill-muted": L.border,
        "--mui-palette-fill-faintHover": L.panel2,
        "--mui-palette-stroke-muted": L.border,
        "--mui-palette-stroke-faint": L.border,
        "--mui-palette-divider": L.border,
        "--mui-palette-secondary-main": L.panel2,
        "--mui-palette-secondary-dark": L.border,
        "--mui-palette-primary-main": L.accent,
        "--mui-palette-primary-mainChannel": channel(L.accent),
        "--mui-palette-primary-dark": L.accent2,
        "--mui-palette-primary-contrastText": L.onAccent,
        "--mui-palette-accent-main": L.accent,
        "--mui-palette-accent-mainChannel": channel(L.accent),
        "--mui-palette-accent-dark": L.accent2,
        "--mui-palette-critical-main": L.err,
        "--mui-palette-critical-mainChannel": channel(L.err),
        "--mui-palette-critical-dark": "#dc2626",
        "--mui-palette-error-main": L.err,
        "--mui-palette-FilledInput-bg": "var(--mui-palette-background-default)",
        "--mui-palette-FilledInput-hoverBg":
            "var(--mui-palette-background-default)",
        "--mui-palette-backdrop-muted": "rgba(0, 0, 0, 0.4)",
        ...theme.applyStyles("dark", {
            "--mui-palette-background-default": D.bg2,
            "--mui-palette-background-paper": D.panel,
            "--mui-palette-background-paper2": D.panel,
            "--mui-palette-text-muted": D.muted,
            "--mui-palette-text-secondary": D.muted,
            "--mui-palette-text-faint": D.faint,
            "--mui-palette-fill-faint": D.bg2,
            "--mui-palette-fill-fainter": D.bg2,
            "--mui-palette-fill-muted": D.panel2,
            "--mui-palette-fill-faintHover": D.bg2,
            "--mui-palette-stroke-muted": D.border,
            "--mui-palette-stroke-faint": D.panel2,
            "--mui-palette-divider": D.panel2,
            "--mui-palette-secondary-main": D.bg2,
            "--mui-palette-secondary-dark": D.bg,
            "--mui-palette-primary-main": D.accent,
            "--mui-palette-primary-mainChannel": channel(D.accent),
            "--mui-palette-primary-dark": D.accent2,
            "--mui-palette-primary-contrastText": D.onAccent,
            "--mui-palette-accent-main": D.accent,
            "--mui-palette-accent-mainChannel": channel(D.accent),
            "--mui-palette-accent-dark": D.accent2,
        }),
        fontFamily: inter,
        "& .MuiTypography-h3:not([data-sidebar-subscription] *)": {
            fontSize: "18px",
            lineHeight: "24px",
        },
        "& .MuiTypography-small.MuiTypography-noWrap": { lineHeight: "16px" },
        "& .MuiTypography-h3 + .MuiTypography-small": {
            fontSize: "13px",
            lineHeight: "18px",
        },
        "& .MuiButton-root": { fontSize: "14px", fontWeight: 600 },
        "& .MuiButton-root:has(> .MuiStack-root):hover": {
            backgroundColor: "var(--mui-palette-background-default)",
        },
        "& .MuiInputBase-root": { fontSize: "14px" },
        "& .MuiInputBase-input::placeholder": {
            color: "var(--mui-palette-text-faint)", // PHOTOVAULT: theme
            opacity: 1,
        },
        "& .MuiIconButton-root:hover": {
            backgroundColor:
                "color-mix(in srgb, var(--mui-palette-text-base) 8%, var(--mui-palette-background-paper))",
        },
        "& .MuiSwitch-root": {
            "--mui-palette-primary-main":
                theme.colorSchemes.light?.palette.primary.main,
            "--mui-palette-fill-muted":
                theme.colorSchemes.light?.palette.fill.muted,
            "--mui-palette-stroke-muted":
                theme.colorSchemes.light?.palette.stroke.muted,
            "--mui-palette-stroke-faint":
                theme.colorSchemes.light?.palette.stroke.faint,
            ...theme.applyStyles("dark", {
                "--mui-palette-primary-main":
                    theme.colorSchemes.dark?.palette.primary.main,
                "--mui-palette-fill-muted":
                    theme.colorSchemes.dark?.palette.fill.muted,
                "--mui-palette-stroke-muted":
                    theme.colorSchemes.dark?.palette.stroke.muted,
                "--mui-palette-stroke-faint":
                    theme.colorSchemes.dark?.palette.stroke.faint,
            }),
        },
        "& .ente-chevron-right:not([data-sidebar-subscription] *)": {
            color: "var(--mui-palette-text-faint)", // PHOTOVAULT: theme
        },
    };
};

export const sidebarTheme = (theme: Theme): Theme => {
    const colors = sidebarStyles(theme);

    return {
        ...theme,
        typography: {
            ...theme.typography,
            fontFamily: inter,
            fontWeightRegular: 400,
            fontWeightMedium: 500,
            h2: {
                ...theme.typography.h2,
                fontFamily: sidebarHeadingFont,
                fontSize: "24px",
                lineHeight: "32px",
                fontWeight: 600,
            },
            h3: { ...theme.typography.h3, fontFamily: inter },
            h5: {
                ...theme.typography.h5,
                fontSize: "18px",
                lineHeight: "24px",
            },
            body: {
                ...theme.typography.body,
                fontFamily: inter,
                fontSize: "14px",
                fontWeight: 400,
            },
            small: {
                ...theme.typography.small,
                fontSize: "12px",
                lineHeight: "16px",
                fontWeight: 400,
            },
            mini: {
                ...theme.typography.mini,
                lineHeight: "16px",
                fontWeight: 400,
            },
        },
        components: {
            ...theme.components,
            MuiDialog: {
                ...theme.components?.MuiDialog,
                styleOverrides: {
                    ...theme.components?.MuiDialog?.styleOverrides,
                    root: [
                        theme.components?.MuiDialog?.styleOverrides?.root,
                        colors,
                    ],
                },
            },
            MuiDrawer: {
                ...theme.components?.MuiDrawer,
                styleOverrides: {
                    ...theme.components?.MuiDrawer?.styleOverrides,
                    root: [
                        theme.components?.MuiDrawer?.styleOverrides?.root,
                        colors,
                    ],
                },
            },
            MuiPopover: {
                ...theme.components?.MuiPopover,
                styleOverrides: {
                    ...theme.components?.MuiPopover?.styleOverrides,
                    root: [
                        theme.components?.MuiPopover?.styleOverrides?.root,
                        colors,
                    ],
                },
            },
        },
    };
};
