// PHOTOVAULT: connect - user-facing text for a server check. The contract
// itself lives in ente-base/photovault/server.
import {
    isLocalNetworkHost,
    serverDisplayName,
    type ServerCheck,
} from "ente-base/photovault/server";
import { t } from "i18next";

/** The success line, or `undefined` when the check failed. */
export const serverCheckSuccessText = (check: ServerCheck) => {
    switch (check.kind) {
        case "photovault":
            return t("pv_server_connected", {
                name: check.info.name,
                version: check.info.version,
            });
        case "ente":
            // Kind "ente" internally; the text avoids the brand (the only
            // user-visible Ente mention is "Based on Ente" in About).
            return t("pv_server_ente_compatible");
        default:
            return undefined;
    }
};

/** The error line, or `undefined` when the check succeeded. */
export const serverCheckErrorText = (check: ServerCheck) => {
    const host = serverDisplayName(check.origin);
    switch (check.kind) {
        case "unreachable": {
            // PHOTOVAULT: connect - a https page can't fetch an http://
            // server (mixed content), so "are you on your home Wi-Fi?" would
            // be misleading.
            if (
                typeof window != "undefined" &&
                window.location.protocol == "https:" &&
                check.origin.startsWith("http:") &&
                // Browsers exempt loopback from mixed-content blocking.
                !/^http:\/\/(localhost|127\.|\[::1\])/i.test(check.origin)
            ) {
                return t("pv_server_mixed_content");
            }
            let local = false;
            try {
                local = isLocalNetworkHost(new URL(check.origin).hostname);
            } catch {
                // Keep the generic text.
            }
            return local
                ? t("pv_server_unreachable_home", { host })
                : t("pv_server_unreachable", { host });
        }
        case "invalid":
            return t("pv_server_invalid_response", { host });
        default:
            return undefined;
    }
};
