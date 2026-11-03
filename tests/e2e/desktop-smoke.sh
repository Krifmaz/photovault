#!/usr/bin/env bash
# Smoke test for the UNMODIFIED upstream Ente Photos desktop app (Electron) at
# the commit in UPSTREAM_COMMIT: builds it like a release (no build-time API
# override), launches it headless under Xvfb, points it at our server through
# the in-app developer settings (seven taps on the landing page) and checks
# that the login screen then talks to our server.
#
# Usage: tests/e2e/desktop-smoke.sh
#   E2E_API_URL  server to point the app at (default http://localhost:18080);
#                it must already be running (e.g. start one as run.sh does).
#
# The build is heavy (Electron, libvips and ffmpeg downloads plus a native
# Rust addon) and is cached in $PHOTOVAULT_E2E_CACHE/desktop-<commit>.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
commit="$(tr -d '[:space:]' < "$here/UPSTREAM_COMMIT")"
cache="${PHOTOVAULT_E2E_CACHE:-${XDG_CACHE_HOME:-$HOME/.cache}/photovault-e2e}"
src="$cache/src-${commit:0:12}"
out="$cache/desktop-${commit:0:12}"
api="${E2E_API_URL:-http://localhost:18080}"
artifacts="$here/artifacts/desktop"
mkdir -p "$artifacts"

# Reuse the web checkout (and its node_modules) from build-web.sh.
PHOTOVAULT_E2E_API="${PHOTOVAULT_E2E_WEB_API:-http://localhost:18080}" "$here/build-web.sh" > /dev/null

# build-web.sh fetched wasm-opt for wasm-pack; the desktop build needs it too.
export PATH="$cache/tools/binaryen-version_117/bin:$PATH"

if [[ ! -f "$out/.complete" ]]; then
    git -C "$src" sparse-checkout add desktop
    (
        cd "$src/desktop"
        npm ci --no-audit --no-fund
        # Upstream's postinstall (vips, ffmpeg, native deps, ONNX Runtime) is
        # run explicitly so the build also works where npm has
        # ignore-scripts=true configured.
        npm run postinstall
        # Release-like renderer: no NEXT_PUBLIC_ENTE_ENDPOINT, so the app
        # starts on Ente's production API until the user changes it.
        env -u NEXT_PUBLIC_ENTE_ENDPOINT npm run build:quick
    )
    unpacked="$(find "$src/desktop/dist" -maxdepth 1 -type d -name 'linux*unpacked' | head -n 1)"
    [[ -n "$unpacked" ]] || { echo "electron-builder produced no unpacked app" >&2; exit 1; }
    rm -rf "$out"
    cp -a "$unpacked" "$out"
    touch "$out/.complete"
fi

exe="$(find "$out" -maxdepth 1 -type f -perm -u+x -name 'ente*' | head -n 1)"
[[ -n "$exe" ]] || { echo "no executable in $out" >&2; exit 1; }

curl -fsS -o /dev/null "$api/ping" || { echo "no server answering $api/ping" >&2; exit 1; }

E2E_API_URL="$api" E2E_ARTIFACTS="$artifacts" \
    xvfb-run -a --server-args="-screen 0 1400x900x24" node "$here/lib/desktop-smoke.mjs" "$exe"
