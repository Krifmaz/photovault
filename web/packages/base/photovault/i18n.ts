// PHOTOVAULT: strip - Photovault rewords a few Ente strings (e.g. "family
// plan" -> "family", since a Photovault server has no plans). The reworded
// `pv_*` strings exist only in the locales that have them (en-US for now);
// everywhere else the user keeps Ente's translated string instead of falling
// back to English in the middle of a translated screen.

import i18n, { t, type TOptions } from "i18next";

/**
 * Translate {@link pvKey} if the current locale has it, otherwise
 * {@link upstreamKey} (Ente's translated string, in the current locale).
 */
export const pvT = (
    pvKey: string,
    upstreamKey: string,
    options?: TOptions,
): string => {
    const lng = i18n.resolvedLanguage ?? i18n.language;
    const has = !!lng && !!i18n.getResource(lng, "translation", pvKey);
    return t(has ? pvKey : upstreamKey, options ?? {});
};
