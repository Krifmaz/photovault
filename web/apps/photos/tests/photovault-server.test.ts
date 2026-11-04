// PHOTOVAULT: connect - server address normalisation, default server
// resolution, the QR payload and the /photovault/info check.
import {
    checkServer,
    companionAppURL,
    connectQRPayload,
    fetchEmailVerificationMode,
    isLocalNetworkHost,
    localServerFallback,
    normalizeServerURL,
    resolveServerOrigin,
    serverDisplayName,
    shouldAutoSubmitEmailCode,
    skippedEmailCode,
} from "ente-base/photovault/server";
import { describe, expect, it } from "vitest";

describe("normalizeServerURL", () => {
    it.each([
        ["10.0.1.7:8095", "http://10.0.1.7:8095"],
        ["  10.0.1.7:8095/  ", "http://10.0.1.7:8095"],
        ["192.168.1.20", "http://192.168.1.20"],
        ["172.20.0.2:8080", "http://172.20.0.2:8080"],
        ["localhost:8080", "http://localhost:8080"],
        ["nas.local:8080", "http://nas.local:8080"],
        ["nas", "http://nas"],
        ["[::1]:8080", "http://[::1]:8080"],
        ["photos.example.com", "https://photos.example.com"],
        ["Photos.Example.COM/", "https://photos.example.com"],
        ["photos.example.com:8443", "http://photos.example.com:8443"],
        ["photos.example.com:443", "https://photos.example.com"],
        ["//photos.example.com", "https://photos.example.com"],
        ["http://photos.example.com", "http://photos.example.com"],
        ["https://photos.example.com:443/", "https://photos.example.com"],
        ["https://example.com/api/", "https://example.com/api"],
        ["https://example.com/api//", "https://example.com/api"],
        ["https://example.com/photovault/info", "https://example.com"],
        ["https://example.com/api/ping", "https://example.com/api"],
        ["https://user:pw@example.com/?x=1#y", "https://example.com"],
        [
            "photovault://connect?url=https%3A%2F%2Fphotos.example.com&home=http%3A%2F%2F10.0.1.7%3A8095&name=Home",
            "https://photos.example.com",
        ],
    ])("%s → %s", (input, expected) => {
        expect(normalizeServerURL(input)).toBe(expected);
    });

    it.each([
        "",
        "   ",
        "ftp://example.com",
        "javascript:alert(1)",
        "mailto:a@example.com",
        "file:///etc/passwd",
        "http://",
        "photovault://connect",
        "photovault://connect?url=photovault%3A%2F%2Fconnect",
        "exa mple.com",
    ])("rejects %j", (input) => {
        expect(normalizeServerURL(input)).toBeUndefined();
    });
});

describe("isLocalNetworkHost", () => {
    it("recognises home-network hosts", () => {
        for (const h of [
            "localhost",
            "127.0.0.1",
            "10.0.1.7",
            "192.168.0.1",
            "172.16.0.1",
            "172.31.255.255",
            "169.254.1.1",
            "nas",
            "nas.local",
            "pi.lan",
            "box.home.arpa",
            "[::1]",
            "[fd00::1]",
        ])
            expect(isLocalNetworkHost(h), h).toBe(true);
    });

    it("treats public hosts as remote", () => {
        for (const h of [
            "photos.example.com",
            "8.8.8.8",
            "172.32.0.1",
            "172.15.0.1",
            "[2001:db8::1]",
        ])
            expect(isLocalNetworkHost(h), h).toBe(false);
    });
});

describe("resolveServerOrigin", () => {
    it("prefers the server saved on this device", () => {
        expect(
            resolveServerOrigin({
                saved: "10.0.1.7:8095",
                buildEnv: "https://build.example.com",
                page: "https://page.example.com",
            }),
        ).toBe("http://10.0.1.7:8095");
    });

    it("then the build-time NEXT_PUBLIC_ENTE_ENDPOINT", () => {
        expect(
            resolveServerOrigin({
                saved: null,
                buildEnv: "https://build.example.com/",
                page: "https://page.example.com",
            }),
        ).toBe("https://build.example.com");
    });

    it("treats an empty build-time endpoint as unset (same origin)", () => {
        expect(
            resolveServerOrigin({
                saved: "",
                buildEnv: "",
                page: "https://photos.example.com",
            }),
        ).toBe("https://photos.example.com");
    });

    it("ignores an unusable saved value", () => {
        expect(
            resolveServerOrigin({
                saved: "ftp://x",
                page: "http://10.0.1.7:8095",
            }),
        ).toBe("http://10.0.1.7:8095");
    });

    it("falls back to localhost when nothing applies", () => {
        expect(resolveServerOrigin({})).toBe(localServerFallback);
        expect(resolveServerOrigin({ page: "ente://app" })).toBe(
            localServerFallback,
        );
    });
});

describe("companion apps and display", () => {
    it("puts albums and cast under the server origin", () => {
        expect(companionAppURL("https://p.example.com", "cast")).toBe(
            "https://p.example.com/cast",
        );
        expect(companionAppURL("https://p.example.com/", "/albums")).toBe(
            "https://p.example.com/albums",
        );
    });

    it("shows addresses without the scheme", () => {
        expect(serverDisplayName("http://10.0.1.7:8095")).toBe("10.0.1.7:8095");
        expect(serverDisplayName("https://example.com/api")).toBe(
            "example.com/api",
        );
    });
});

describe("connectQRPayload", () => {
    it("encodes the contract's photovault://connect URL", () => {
        expect(
            connectQRPayload({
                publicUrl: "https://photos.example.com",
                homeUrl: "http://10.0.1.7:8095",
                name: "Our photos",
            }),
        ).toBe(
            "photovault://connect?url=https%3A%2F%2Fphotos.example.com&home=http%3A%2F%2F10.0.1.7%3A8095&name=Our%20photos",
        );
    });

    it("omits home when the server has none", () => {
        expect(
            connectQRPayload({
                publicUrl: "https://p.example.com",
                homeUrl: null,
                name: "P",
            }),
        ).toBe("photovault://connect?url=https%3A%2F%2Fp.example.com&name=P");
    });

    it("round-trips through normalizeServerURL", () => {
        const payload = connectQRPayload({
            publicUrl: "https://p.example.com",
            homeUrl: null,
            name: "P",
        });
        expect(normalizeServerURL(payload)).toBe("https://p.example.com");
    });
});

describe("checkServer", () => {
    const info = {
        product: "photovault",
        name: "Photovault",
        version: "0.2.0",
        publicUrl: "https://photos.example.com",
        homeUrl: "http://10.0.1.7:8095",
        signupsAllowed: false,
        someFutureField: 1,
    };

    const json = (body: unknown, status = 200) =>
        new Response(JSON.stringify(body), { status });

    it("reads /photovault/info", async () => {
        const urls: string[] = [];
        const fetchFn = ((url: string) => {
            urls.push(url);
            return Promise.resolve(json(info));
        }) as unknown as typeof fetch;
        const check = await checkServer("http://10.0.1.7:8095", { fetchFn });
        expect(urls).toEqual(["http://10.0.1.7:8095/photovault/info"]);
        expect(check.kind).toBe("photovault");
        if (check.kind == "photovault") {
            expect(check.info.name).toBe("Photovault");
            expect(check.info.version).toBe("0.2.0");
        }
    });

    it("falls back to /ping for a stock Ente server", async () => {
        const fetchFn = ((url: string) =>
            Promise.resolve(
                url.endsWith("/ping")
                    ? json({ message: "pong", id: "x" })
                    : json({ code: "NOT_FOUND" }, 404),
            )) as unknown as typeof fetch;
        expect((await checkServer("https://e.example", { fetchFn })).kind).toBe(
            "ente",
        );
    });

    it("reports network errors as unreachable", async () => {
        const fetchFn = (() =>
            Promise.reject(new TypeError("Failed to fetch"))) as typeof fetch;
        expect(
            (await checkServer("http://10.0.1.7:8095", { fetchFn })).kind,
        ).toBe("unreachable");
    });

    it("reports a timeout as unreachable", async () => {
        const fetchFn = ((_: string, init?: RequestInit) =>
            new Promise((_resolve, reject) => {
                init?.signal?.addEventListener("abort", () =>
                    reject(new DOMException("aborted", "AbortError")),
                );
            })) as unknown as typeof fetch;
        const check = await checkServer("http://10.0.1.7:8095", {
            fetchFn,
            timeoutMs: 10,
        });
        expect(check.kind).toBe("unreachable");
    });

    it("rejects something that is not a Photovault or Ente server", async () => {
        const html = (() =>
            Promise.resolve(
                new Response("<html></html>", { status: 200 }),
            )) as typeof fetch;
        expect(
            (await checkServer("https://x.example", { fetchFn: html })).kind,
        ).toBe("invalid");
        const wrongProduct = (() =>
            Promise.resolve(
                json({ ...info, product: "other" }),
            )) as typeof fetch;
        expect(
            (await checkServer("https://x.example", { fetchFn: wrongProduct }))
                .kind,
        ).toBe("invalid");
    });

    it("requires product and signupsAllowed", async () => {
        const { product: _p, ...noProduct } = info;
        const { signupsAllowed: _s, ...noSignups } = info;
        for (const body of [noProduct, noSignups]) {
            const fetchFn = (() =>
                Promise.resolve(json(body))) as unknown as typeof fetch;
            expect(
                (await checkServer("https://x.example", { fetchFn })).kind,
            ).toBe("invalid");
        }
    });
});

// PHOTOVAULT: email-skip - when the client submits the email code itself.
describe("shouldAutoSubmitEmailCode", () => {
    it("auto-submits only for skip", () => {
        expect(shouldAutoSubmitEmailCode("skip")).toBe(true);
        for (const mode of ["smtp", "log", "future", "", null, undefined])
            expect(shouldAutoSubmitEmailCode(mode)).toBe(false);
    });

    it("keeps the code screen for accounts with email 2FA", () => {
        expect(
            shouldAutoSubmitEmailCode("skip", { isEmailMFAEnabled: true }),
        ).toBe(false);
        expect(
            shouldAutoSubmitEmailCode("skip", { isEmailMFAEnabled: false }),
        ).toBe(true);
    });

    it("sends a 6-digit code", () => {
        expect(skippedEmailCode).toMatch(/^\d{6}$/);
    });
});

describe("fetchEmailVerificationMode", () => {
    const info = {
        product: "photovault",
        name: "Photovault",
        version: "0.2.0",
        publicUrl: "https://photos.example.com",
        homeUrl: null,
        signupsAllowed: true,
    };
    const respond = (body: unknown, status = 200) =>
        (() =>
            Promise.resolve(
                new Response(JSON.stringify(body), { status }),
            )) as unknown as typeof fetch;

    it("reads emailVerification from /photovault/info only", async () => {
        const urls: string[] = [];
        const fetchFn = ((url: string) => {
            urls.push(url);
            return Promise.resolve(
                new Response(
                    JSON.stringify({ ...info, emailVerification: "skip" }),
                ),
            );
        }) as unknown as typeof fetch;
        expect(
            await fetchEmailVerificationMode("http://10.0.1.7:8095", {
                fetchFn,
            }),
        ).toBe("skip");
        expect(urls).toEqual(["http://10.0.1.7:8095/photovault/info"]);
    });

    it("is undefined when the field is absent", async () => {
        expect(
            await fetchEmailVerificationMode("https://x.example", {
                fetchFn: respond(info),
            }),
        ).toBeUndefined();
    });

    it("is undefined for a stock Ente server, errors and timeouts", async () => {
        expect(
            await fetchEmailVerificationMode("https://e.example", {
                fetchFn: respond({ code: "NOT_FOUND" }, 404),
            }),
        ).toBeUndefined();
        expect(
            await fetchEmailVerificationMode("https://x.example", {
                fetchFn: () => Promise.reject(new TypeError("x")),
            }),
        ).toBeUndefined();
        const hang = ((_: string, init?: RequestInit) =>
            new Promise((_resolve, reject) => {
                init?.signal?.addEventListener("abort", () =>
                    reject(new DOMException("aborted", "AbortError")),
                );
            })) as unknown as typeof fetch;
        expect(
            await fetchEmailVerificationMode("https://x.example", {
                fetchFn: hang,
                timeoutMs: 10,
            }),
        ).toBeUndefined();
    });

    it("keeps an unknown mode (which then shows the code screen)", async () => {
        const mode = await fetchEmailVerificationMode("https://x.example", {
            fetchFn: respond({ ...info, emailVerification: "future" }),
        });
        expect(mode).toBe("future");
        expect(shouldAutoSubmitEmailCode(mode)).toBe(false);
    });
});
