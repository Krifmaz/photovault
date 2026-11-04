// PHOTOVAULT: connect - the one place that knows the Photovault server
// contract (contracts/server-info.md): the `/photovault/info` model, how a
// typed server address is normalised, which server the app talks to by
// default, and the QR "connect" payload. Pure functions only (no storage, no
// React), so it can be imported from workers and unit tested. If the contract
// changes, change this file.

import { z } from "zod";

/** Path of the public server-info endpoint (no auth, CORS-open). */
export const serverInfoPath = "/photovault/info";

/** Used when nothing else names a server (e.g. an unconfigured desktop app). */
export const localServerFallback = "http://localhost:8080";

/**
 * localStorage key under which the server picked on the login page is kept.
 * Logout preserves it (see `ente-base/local-storage`).
 */
export const savedServerStorageKey = "photovaultServer";

/**
 * `GET /photovault/info`. Required fields per the contract are `product`
 * (must be `"photovault"`), `name`, `version`, `publicUrl` and
 * `signupsAllowed`; a 200 without them is "invalid". `homeUrl` is required
 * but may be null (absent is tolerated). Every other field is optional and
 * unknown fields are ignored.
 */
export const ServerInfo = z.object({
    product: z.literal("photovault"),
    name: z.string(),
    version: z.string(),
    commit: z.string().nullish(),
    apiVersion: z.number().optional(),
    publicUrl: z.string(),
    homeUrl: z.string().nullish(),
    albumsUrl: z.string().nullish(),
    castUrl: z.string().nullish(),
    signupsAllowed: z.boolean(),
    features: z.array(z.string()).optional(),
});

export type ServerInfo = z.infer<typeof ServerInfo>;

/**
 * Result of checking a server address.
 *
 * - "photovault": `/photovault/info` answered with a valid body.
 * - "ente": no `/photovault/info`, but `/ping` answered like Ente's museum.
 * - "unreachable": network error or timeout (nothing answered).
 * - "invalid": something answered, but it is not a Photovault/Ente server.
 */
export type ServerCheck =
    | { kind: "photovault"; origin: string; info: ServerInfo }
    | { kind: "ente"; origin: string }
    | { kind: "unreachable"; origin: string }
    | { kind: "invalid"; origin: string };

const privateHostRE =
    /^(localhost|127\.\d+\.\d+\.\d+|10\.\d+\.\d+\.\d+|192\.168\.\d+\.\d+|172\.(1[6-9]|2\d|3[01])\.\d+\.\d+|169\.254\.\d+\.\d+|\[(::1|f[cd][0-9a-f:]*|fe80:[0-9a-f:]*)\])$/i;

/**
 * True for addresses that are only reachable on a home network: loopback,
 * RFC 1918 / link-local IPs, unique-local IPv6, single-label names and
 * `.local`, `.lan`, `.home.arpa`, `.internal` names.
 */
export const isLocalNetworkHost = (hostname: string) => {
    const h = hostname.toLowerCase();
    return (
        privateHostRE.test(h) ||
        (!h.includes(".") && !h.startsWith("[")) ||
        /\.(local|lan|home|internal|home\.arpa)$/.test(h)
    );
};

/**
 * Turn what someone typed (or scanned) into a server origin, or `undefined`
 * if it is not usable.
 *
 * - Surrounding whitespace is ignored.
 * - Without a scheme, home-network hosts (IPs, `localhost`, `.local`, ...)
 *   and hosts with an explicit non-443 port get `http://`, everything else
 *   `https://`. Only http and https are accepted.
 * - A `photovault://connect?url=...` QR payload yields its `url`.
 * - Credentials, query and fragment are dropped, as are trailing slashes and a
 *   pasted `/photovault/info` or `/ping` suffix. A path prefix (e.g. a server
 *   behind `/api`) is kept, because API paths are appended to it.
 */
export const normalizeServerURL = (input: string): string | undefined => {
    let s = input.trim();
    if (!s) return undefined;

    if (/^photovault:/i.test(s)) {
        try {
            const inner = new URL(s).searchParams.get("url");
            return inner && !/^photovault:/i.test(inner)
                ? normalizeServerURL(inner)
                : undefined;
        } catch {
            return undefined;
        }
    }

    if (!/^[a-z][a-z0-9+.-]*:\/\//i.test(s)) {
        if (/^[a-z][a-z0-9+.-]*:(?!\d)/i.test(s)) return undefined; // e.g. "ftp:x"
        if (s.startsWith("//")) s = s.slice(2);
        let scheme = "https://";
        try {
            const probe = new URL(`http://${s}`);
            if (
                isLocalNetworkHost(probe.hostname) ||
                (probe.port && probe.port != "443")
            ) {
                scheme = "http://";
            }
        } catch {
            return undefined;
        }
        s = scheme + s;
    }

    let url: URL;
    try {
        url = new URL(s);
    } catch {
        return undefined;
    }
    if (url.protocol != "http:" && url.protocol != "https:") return undefined;
    if (!url.hostname) return undefined;

    let path = url.pathname.replace(/\/+$/, "");
    path = path.replace(/\/(photovault\/info|ping)$/i, "");
    return url.origin + path;
};

/** The address without the scheme, as shown to people ("10.0.1.7:8095"). */
export const serverDisplayName = (origin: string) =>
    origin.replace(/^https?:\/\//i, "").replace(/\/+$/, "");

/** The origin of the current page or worker, when served over http(s). */
export const pageOrigin = (): string | undefined => {
    if (!("location" in globalThis)) return undefined;
    const origin = globalThis.location.origin;
    return /^https?:\/\//.test(origin) ? origin : undefined;
};

/**
 * Which server to talk to, in order: the one chosen on this device, the
 * build-time `NEXT_PUBLIC_ENTE_ENDPOINT` (empty = unset), the origin the web
 * app was served from (one container serves API and web app), then
 * {@link localServerFallback}.
 */
export const resolveServerOrigin = ({
    saved,
    buildEnv,
    page,
}: {
    saved?: string | null;
    buildEnv?: string | null;
    page?: string | null;
}): string =>
    (saved && normalizeServerURL(saved)) ||
    (buildEnv && normalizeServerURL(buildEnv)) ||
    (page && normalizeServerURL(page)) ||
    localServerFallback;

/**
 * Default origin of a companion web app served by the same server under
 * `subPath` (contracts/apps.json: `/albums`, `/cast`, `/accounts`).
 */
export const companionAppURL = (serverOrigin: string, subPath: string) =>
    `${serverOrigin.replace(/\/+$/, "")}/${subPath.replace(/^\/+/, "")}`;

/**
 * The QR payload of the Settings → Server page
 * (`photovault://connect?url=&home=&name=`), from contracts/server-info.md.
 */
export const connectQRPayload = (
    info: Pick<ServerInfo, "publicUrl" | "homeUrl" | "name">,
) => {
    const params = [`url=${encodeURIComponent(info.publicUrl)}`];
    if (info.homeUrl) params.push(`home=${encodeURIComponent(info.homeUrl)}`);
    params.push(`name=${encodeURIComponent(info.name)}`);
    return `photovault://connect?${params.join("&")}`;
};

const PingResponse = z.object({ message: z.literal("pong") });

/**
 * Check a (normalised) server origin: `GET /photovault/info`, falling back to
 * Ente's `GET /ping` when the server is a stock museum (404).
 */
export const checkServer = async (
    origin: string,
    {
        timeoutMs = 8000,
        fetchFn = globalThis.fetch.bind(globalThis),
    }: { timeoutMs?: number; fetchFn?: typeof fetch } = {},
): Promise<ServerCheck> => {
    const get = async (path: string) => {
        const controller = new AbortController();
        const timer = setTimeout(() => controller.abort(), timeoutMs);
        try {
            return await fetchFn(origin + path, {
                signal: controller.signal,
                cache: "no-store",
            });
        } finally {
            clearTimeout(timer);
        }
    };

    let res: Response;
    try {
        res = await get(serverInfoPath);
    } catch {
        return { kind: "unreachable", origin };
    }

    if (res.ok) {
        try {
            const info = ServerInfo.parse(await res.json());
            return { kind: "photovault", origin, info };
        } catch {
            return { kind: "invalid", origin };
        }
    }

    if (res.status == 404) {
        try {
            const ping = await get("/ping");
            if (ping.ok && PingResponse.safeParse(await ping.json()).success)
                return { kind: "ente", origin };
        } catch {
            return { kind: "unreachable", origin };
        }
    }
    return { kind: "invalid", origin };
};
