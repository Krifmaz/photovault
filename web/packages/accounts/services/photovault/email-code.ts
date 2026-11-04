// PHOTOVAULT: email-skip - skip the "Check your inbox" screen on servers that
// do not email codes (`emailVerification: "skip"` in /photovault/info, see
// contracts/server-info.md). Sign-up and email-code login ask the server for
// its mode alongside `POST /users/ott` and, when the code can be skipped,
// leave a note for /verify, which then submits `skippedEmailCode` through its
// normal success path (and shows the code screen if that fails). Only email
// verification is affected; keys, SRP and 2FA flows are unchanged.

import { apiOrigin } from "ente-base/origins";
import { fetchEmailVerificationMode } from "ente-base/photovault/server";

const sessionKey = "photovaultSkipEmailCode";

/**
 * The configured server's `emailVerification` mode, or `undefined` (stock
 * Ente server, error, timeout). Never throws.
 */
export const serverEmailVerificationMode = async () => {
    try {
        return await fetchEmailVerificationMode(await apiOrigin());
    } catch {
        return undefined;
    }
};

/** Note that /verify may submit the code itself for `email`. */
export const stashSkipEmailCode = (email: string) => {
    try {
        sessionStorage.setItem(sessionKey, email);
    } catch {
        // No session storage: the code screen is shown, as upstream.
    }
};

/**
 * True (once) when sign-up or login left the note for this `email`. The note
 * is removed either way, so a reload of /verify shows the code screen.
 */
export const unstashSkipEmailCode = (email: string) => {
    try {
        const stashed = sessionStorage.getItem(sessionKey);
        sessionStorage.removeItem(sessionKey);
        return stashed == email;
    } catch {
        return false;
    }
};
