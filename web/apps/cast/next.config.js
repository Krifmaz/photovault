const baseConfig = require("ente-base/next.config.base.js");

// PHOTOVAULT: connect - the Photovault server serves this app from the same
// origin as the API under /cast/ (contracts/apps.json). Set
// PHOTOVAULT_CAST_BASE_PATH="" to build it for its own host (Ente's layout).
const pvBasePath = process.env.PHOTOVAULT_CAST_BASE_PATH ?? "/cast";

module.exports = {
    ...baseConfig,
    ...(pvBasePath && { basePath: pvBasePath }),
    env: { ...baseConfig.env, photovaultBasePath: pvBasePath },
};
