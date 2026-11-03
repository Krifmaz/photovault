#!/usr/bin/env bash
# Build the UNMODIFIED upstream Ente Photos (and public Albums) web apps at the
# commit pinned in UPSTREAM_COMMIT, pointed at our server via the official
# self-hosting build variable NEXT_PUBLIC_ENTE_ENDPOINT. No source changes.
#
# Output (cached outside the repo, reused when commit + endpoint are unchanged):
#   $PHOTOVAULT_E2E_CACHE/web-<commit>-<endpoint-slug>/{photos,albums}
# Prints the output directory on the last line of stdout.
#
# Requirements: git, node 22+ / npm, Rust with the wasm32-unknown-unknown
# target (the web apps compile parts of Ente's Rust code to WebAssembly).
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
commit="$(tr -d '[:space:]' < "$here/UPSTREAM_COMMIT")"
endpoint="${PHOTOVAULT_E2E_API:-http://localhost:18080}"
cache="${PHOTOVAULT_E2E_CACHE:-${XDG_CACHE_HOME:-$HOME/.cache}/photovault-e2e}"
upstream_url="${PHOTOVAULT_E2E_UPSTREAM:-https://github.com/ente-io/ente.git}"

case "$commit" in
    *[!0-9a-f]* | "") echo "UPSTREAM_COMMIT must be a hex commit id" >&2; exit 2 ;;
esac

slug="$(printf '%s' "$endpoint" | tr -c 'A-Za-z0-9' '_')"
out="$cache/web-${commit:0:12}-$slug"
src="$cache/src-${commit:0:12}"

if [[ -f "$out/.complete" ]]; then
    echo "web build cached: $out" >&2
    echo "$out"
    exit 0
fi

mkdir -p "$cache"

if [[ ! -f "$src/.checked-out" ]]; then
    rm -rf "$src"
    git init -q "$src"
    git -C "$src" remote add origin "$upstream_url"
    git -C "$src" config core.sparseCheckout true
    git -C "$src" sparse-checkout set web rust
    git -C "$src" fetch -q --depth 1 --filter=blob:none origin "$commit"
    git -C "$src" checkout -q FETCH_HEAD
    touch "$src/.checked-out"
fi

got="$(git -C "$src" rev-parse HEAD)"
if [[ "$got" != "$commit" ]]; then
    echo "checkout is at $got, expected $commit" >&2
    exit 1
fi

rustup target add wasm32-unknown-unknown >/dev/null 2>&1 || true

# wasm-pack downloads wasm-opt (binaryen) itself, but its downloader ignores
# the system CA store, which fails behind TLS-intercepting proxies. It uses a
# wasm-opt found on PATH instead, so fetch the version it pins with curl.
binaryen_ver=version_117
binaryen_dir="$cache/tools/binaryen-$binaryen_ver"
if ! command -v wasm-opt >/dev/null 2>&1; then
    if [[ ! -x "$binaryen_dir/bin/wasm-opt" ]]; then
        mkdir -p "$cache/tools"
        curl -fsSL -o "$cache/tools/binaryen.tgz" \
            "https://github.com/WebAssembly/binaryen/releases/download/$binaryen_ver/binaryen-$binaryen_ver-x86_64-linux.tar.gz"
        tar -xzf "$cache/tools/binaryen.tgz" -C "$cache/tools"
        rm -f "$cache/tools/binaryen.tgz"
    fi
    export PATH="$binaryen_dir/bin:$PATH"
fi

start=$SECONDS
(
    cd "$src/web"
    if [[ ! -f node_modules/.photovault-ci ]]; then
        npm ci --no-audit --no-fund
        touch node_modules/.photovault-ci
    fi
    export NEXT_PUBLIC_ENTE_ENDPOINT="$endpoint"
    export NEXT_TELEMETRY_DISABLED=1
    npm run build:photos
    npm run build:albums
)
rm -rf "$out.tmp"
mkdir -p "$out.tmp"
cp -a "$src/web/apps/photos/out" "$out.tmp/photos"
cp -a "$src/web/apps/albums/out" "$out.tmp/albums"
printf 'commit=%s\nendpoint=%s\nbuild_seconds=%s\n' "$commit" "$endpoint" "$((SECONDS - start))" > "$out.tmp/BUILD_INFO"
touch "$out.tmp/.complete"
rm -rf "$out"
mv "$out.tmp" "$out"
echo "web build done in $((SECONDS - start))s: $out" >&2
echo "$out"
