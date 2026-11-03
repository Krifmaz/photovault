#!/bin/sh
# PHOTOVAULT: strip (new file).
#
# Ente's web app loads a few static data files from Ente's CDN
# (assets.ente.com). Photovault loads them from its own origin instead, under
# /static-assets/ (see `staticAssetURL` in packages/base/origins.ts), so the
# browser never contacts Ente. This script copies them into the Photos app's
# public folder before `next build`, so they end up in the static export (and in
# the desktop app, which bundles that export).
#
# Without them the app still works; only these features degrade:
#   - Places (photos grouped by city): location/v2/{cities,urban-centres}.bin
#   - City names in search:            world_cities.json
#   - Video processing in the browser: ffmpeg-core-9.0.2/ffmpeg-core.{js,wasm}
#     (the desktop app uses its bundled native ffmpeg instead)
#
# Usage: scripts/photovault-vendor-assets.sh [SOURCE]
#   SOURCE is a base URL or a local directory with the same layout. The default
#   is Ente's CDN, fetched once at build time (never by users' browsers).
# No arguments are passed to a shell; paths are fixed below.

set -eu

src="${1:-https://assets.ente.com}"
dest="$(dirname "$0")/../apps/photos/public/static-assets"

files="
location/v2/cities.bin
location/v2/urban-centres.bin
world_cities.json
ffmpeg-core-9.0.2/ffmpeg-core.js
ffmpeg-core-9.0.2/ffmpeg-core.wasm
"

for f in $files; do
    mkdir -p "$dest/$(dirname "$f")"
    case "$src" in
        http://* | https://*)
            curl --fail --silent --show-error --location \
                --output "$dest/$f" "$src/$f"
            ;;
        *)
            cp "$src/$f" "$dest/$f"
            ;;
    esac
    echo "vendored $f"
done
