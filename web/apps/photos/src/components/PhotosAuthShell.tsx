import { AuthPageShell } from "ente-accounts/components/auth/AuthPageShell";
import type { AuthPresentationConfig } from "ente-accounts/components/auth/styles";
import { isDesktop } from "ente-base/app";
// PHOTOVAULT: theme
import {
    pvDark,
    pvGradient,
    pvLight,
    type PVScheme,
} from "ente-base/components/utils/photovault/encore-tokens";
import { t } from "i18next";
import type React from "react";

interface PhotosAuthShellProps extends React.PropsWithChildren {
    contentWidth?: 400 | 420;
}

// PHOTOVAULT: theme - Encore colours for the sign-in pages (from the tokens).
const pvAuthScheme = (c: PVScheme) => ({
    page: c.bg,
    card: c.panel,
    brand: pvDark.bg2,
    "brand-text": pvDark.text,
    "brand-text-muted": pvDark.muted,
    primary: c.accent,
    "primary-hover": c.accent2,
    "primary-active": c.accent2,
    "primary-gradient": pvGradient(c),
    "on-primary": c.onAccent,
    focus: c.accent2,
    link: c.accent2,
    "terms-link": c.accent2,
    success: c.ok,
    warning: c.err,
    caution: c.warn,
    text: c.text,
    "text-muted": c.muted,
    "text-faint": c.faint,
    field: c.bg2,
    fill: c.panel2,
    "fill-hover": c.border,
    "fill-active": c.border,
    stroke: c.border,
    "button-secondary": c.panel2,
    "button-secondary-hover": c.border,
    "button-secondary-active": c.border,
});

const photosAuthTheme: AuthPresentationConfig = {
    primary: pvLight.accent,
    primaryHover: pvLight.accent2,
    primaryActive: pvLight.accent2,
    focus: pvLight.accent2,
    focusDark: pvDark.accent2,
    link: pvLight.accent2,
    linkDark: pvDark.accent2,
    termsLink: pvLight.accent2,
    termsLinkDark: pvDark.accent2,
    passwordMessage: pvLight.text,
    passwordMessageDark: pvDark.text,
    textDisabledDark: pvDark.faint,
    mobileBrandHeight: 196,
    illustrationHeights: { desktop: 219, tablet: 172, mobile: 127, wide: 249 },
    photovault: { light: pvAuthScheme(pvLight), dark: pvAuthScheme(pvDark) },
};

export function PhotosAuthShell({
    children,
    contentWidth,
}: PhotosAuthShellProps): React.JSX.Element {
    return (
        <AuthPageShell
            brand={{
                headline: t("photos_auth_headline"),
                subtitle: t("photos_auth_subtitle"),
                // PHOTOVAULT: strip - instead of Ente's "10 GB free forever"
                // plan and "Independently audited" (Ente's audit) claims.
                bullets: [
                    t("pv_auth_own_server"),
                    t("photos_auth_no_ads_no_spying"),
                    t("pv_auth_your_keys"),
                ],
            }}
            presentationConfig={photosAuthTheme}
            contentWidth={contentWidth}
            viewportHeight={
                isDesktop
                    ? "calc(100svh - env(titlebar-area-height, 30px))"
                    : undefined
            }
        >
            {children}
        </AuthPageShell>
    );
}
