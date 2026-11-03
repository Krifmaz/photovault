import { getKVS } from "ente-base/kv";
import { buildEnvCustomAPIEndpoint } from "./env";

// PHOTOVAULT: strip - never fall back to Ente's servers. Every Photovault
// deployment is a "custom" (self-hosted) origin in Ente's terms, so the
// self-hosting code paths Ente already has are the ones that run, and the
// Ente-hosted CDN, uploader, payments and preview hosts are never contacted.
//
// Order: the server saved on this device (Ente's developer settings / server
// connection), then the build-time `NEXT_PUBLIC_ENTE_ENDPOINT` (empty means
// "the origin this page was served from", see contracts/apps.json), then the
// page's own origin when it is http(s). Only when none applies (e.g. a desktop
// build that has not been pointed at a server yet) is the local museum default
// used.
const localServerFallback = "http://localhost:8080";

/** The page's (or worker's) own origin, when it was served over http(s). */
const sameOriginAPI = (): string | undefined => {
    // Not defined during static rendering.
    if (!("location" in globalThis)) return undefined;
    const origin = globalThis.location.origin;
    return /^https?:\/\//.test(origin) ? origin : undefined;
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
export const customAPIOrigin = async (): Promise<string> =>
    (await getKVS("apiOrigin")) ||
    buildEnvCustomAPIEndpoint ||
    sameOriginAPI() ||
    localServerFallback;

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
