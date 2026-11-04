const baseConfig = require("ente-base/next.config.base.js");

// PHOTOVAULT: connect - the Photovault server serves this app from the same
// origin as the API under /embed/ (the "Embed" option of public links). Set
// PHOTOVAULT_EMBED_BASE_PATH="" to build it for its own host (Ente's layout).
const pvBasePath = process.env.PHOTOVAULT_EMBED_BASE_PATH ?? "/embed";

module.exports = {
    ...baseConfig,
    // PHOTOVAULT: connect
    ...(pvBasePath && { basePath: pvBasePath }),
    env: { ...baseConfig.env, photovaultBasePath: pvBasePath },
    // Keep static export in production; in development, serve path-token links
    // through the index page so local browser loads match Cloudflare fallback.
    ...(process.env.NODE_ENV === "development" && {
        output: undefined,
        async rewrites() {
            return {
                fallback: [
                    {
                        source: "/:path((?!_next|images|favicon.ico).*)",
                        destination: "/",
                    },
                ],
            };
        },
    }),
};
