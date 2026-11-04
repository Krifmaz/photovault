import { getKVS, removeKV, setKV } from "ente-base/kv";
// PHOTOVAULT: connect
import {
    pageOrigin,
    resolveServerOrigin,
    savedServerStorageKey,
} from "ente-base/photovault/server";
import { buildEnvCustomAPIEndpoint } from "./env";

// PHOTOVAULT: strip - never fall back to Ente's servers. Every Photovault
// deployment is a "custom" (self-hosted) origin in Ente's terms, so the
// self-hosting code paths Ente already has are the ones that run, and the
// Ente-hosted CDN, uploader, payments and preview hosts are never contacted.
//
// PHOTOVAULT: connect - the order (server chosen on this device, then
// `NEXT_PUBLIC_ENTE_ENDPOINT`, then the page's own origin, then
// http://localhost:8080) lives in `ente-base/photovault/server`.

/** The server chosen on the login page, kept in localStorage (main thread). */
const savedServerOrigin = (): string | undefined => {
    try {
        return (
            globalThis.localStorage.getItem(savedServerStorageKey) ?? undefined
        );
    } catch {
        return undefined;
    }
};

export const apiOrigin = async () => customAPIOrigin();

// path must begin with "/".
export const apiURL = async (
    path: string,
    queryParams?: Record<string, string | number | boolean>,
) => {
    let url = (await apiOrigin()) + path;
    if (queryParams) {
        const stringQP = Object.fromEntries(
            Object.entries(queryParams).map(([k, v]) => [k, v.toString()]),
        );
        const params = new URLSearchParams(stringQP);
        url = `${url}?${params.toString()}`;
    }
    return url;
};

// PHOTOVAULT: strip - never undefined, so the Ente-hosted branches that callers
// take for "no custom origin" (thumbnails/files/public-albums CDN hosts) are
// unreachable.
export const customAPIOrigin = async (): Promise<string> => {
    // PHOTOVAULT: connect - localStorage survives logout (KV DB does not);
    // Ente's KV "apiOrigin" is what workers can read, so keep it in sync.
    const local = savedServerOrigin();
    const kv = await getKVS("apiOrigin");
    if (local && local != kv) {
        try {
            await setKV("apiOrigin", local);
        } catch {
            // Best effort; the main thread still uses `local`.
        }
    }
    return resolveServerOrigin({
        saved: local ?? kv,
        buildEnv: buildEnvCustomAPIEndpoint,
        page: pageOrigin(),
    });
};

/**
 * PHOTOVAULT: connect - remember (or with `undefined`, forget) the server
 * chosen on this device. Forgetting reverts to the build-time / same-origin
 * default.
 */
export const saveServerOrigin = async (origin: string | undefined) => {
    try {
        if (origin) localStorage.setItem(savedServerStorageKey, origin);
        else localStorage.removeItem(savedServerStorageKey);
    } catch {
        // localStorage unavailable; the KV entry below still applies.
    }
    if (origin) await setKV("apiOrigin", origin);
    else await removeKV("apiOrigin");
};

// PHOTOVAULT: strip - always true (see above), so build-time checks take the
// self-hosted branch.
export const isCustomAPIOrigin = true as boolean;

export const customAPIHost = async () => {
    const origin = await customAPIOrigin();
    return origin ? new URL(origin).host : undefined;
};

// PHOTOVAULT: strip - uploads go to the configured server, never to Ente's
// uploader worker.
export const uploaderOrigin = async () => apiOrigin();

// PHOTOVAULT: strip - static data that Ente serves from assets.ente.com (the
// ffmpeg.wasm core, the world cities list and the places index) is served from
// the web app's own origin instead, under /static-assets/ (same layout as
// Ente's CDN). See web/scripts/photovault-vendor-assets.sh.
export const staticAssetURL = (path: string) => `/static-assets/${path}`;
