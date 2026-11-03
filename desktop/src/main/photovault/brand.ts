/**
 * PHOTOVAULT: brand name and Encore colour tokens for the Electron shell.
 *
 * The in-app UI is themed by the web app (`web/`); these values only cover
 * what the main process paints before the renderer loads (window background,
 * Windows/Linux title bar overlay) and native labels. Colours mirror
 * `branding/encore-tokens.json` (`dark.bg`/`dark.text`, `light.bg`/`light.text`).
 */

/** User-visible application name. */
export const appDisplayName = "Photovault";

/** Credit line required by the fork (shown in the About panel). */
export const upstreamCredit = "Based on Ente (ente.com)";

/** Window background colour before the web app paints. */
export const windowBackgroundColor = (isDark: boolean) =>
    isDark ? "#161617" : "#f6f6fb";

/** Colour of the window control symbols in the title bar overlay. */
export const titleBarSymbolColor = (isDark: boolean) =>
    isDark ? "#f0ead6" : "#16161f";
