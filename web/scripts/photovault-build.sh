#!/usr/bin/env bash
# PHOTOVAULT: build (new file).
#
# Builds the static web apps the Photovault server serves from one directory
# (contracts/apps.json, web/PHOTOVAULT.md "Serving the web apps"):
#
#   web/apps/photos/out    -> <web dir>/            (Photos, the main app)
#   web/apps/albums/out    -> <web dir>/albums/     (public albums, basePath /albums)
#   web/apps/cast/out      -> <web dir>/cast/       (cast receiver, basePath /cast)
#   web/apps/accounts/out  -> <web dir>/accounts/   (passkeys, family, recovery)
#
# Steps: npm ci, the wasm packages (prelogin, photos, cast), the static data
# files (scripts/photovault-vendor-assets.sh), then `next build` of each app
# with the repo's own workspace commands.
#
# Environment (all optional):
#   NEXT_PUBLIC_ENTE_ENDPOINT   API origin baked in as the default server.
#                               Default: empty = the origin the page is served
#                               from (one container serves API and web app).
#   PHOTOVAULT_APPS             Apps to build, default "photos albums cast accounts".
#   PHOTOVAULT_SKIP_NPM_CI=1    Reuse the existing node_modules.
#   PHOTOVAULT_SKIP_WASM=1      Reuse the existing wasm pkg/ folders.
#   PHOTOVAULT_WASM_NO_OPT=1    Build wasm without wasm-opt (debugging only;
#                               larger files).
#   PHOTOVAULT_ASSETS_SOURCE    Base URL or directory for the static data files
#                               (default: Ente's CDN, fetched here once at build
#                               time). PHOTOVAULT_SKIP_ASSETS=1 skips them
#                               (places, city search and in-browser ffmpeg then
#                               do not work).
#   PHOTOVAULT_WEB_OUT          If set, also assemble the served layout there
#                               (photos at the root, sub-apps in subfolders).
#
# Requirements: node 22+ / npm, Rust with the wasm32-unknown-unknown target,
# wasm-pack, and for the cast wasm the nightly toolchain pinned in
# rust/bindings/wasm/cast/rust-toolchain.toml (rustup installs it on first use).
#
# Nothing typed by a user reaches a shell here; inputs are fixed paths and
# build-time environment variables.

set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
web="$(cd "$here/.." && pwd)"
repo="$(cd "$web/.." && pwd)"

apps="${PHOTOVAULT_APPS:-photos albums cast accounts}"
export NEXT_PUBLIC_ENTE_ENDPOINT="${NEXT_PUBLIC_ENTE_ENDPOINT:-}"
export NEXT_TELEMETRY_DISABLED=1

log() { printf '\n==> %s\n' "$*" >&2; }

for app in $apps; do
    case "$app" in
        photos | albums | cast | accounts) ;;
        *) echo "unknown app: $app (expected photos, albums, cast, accounts)" >&2; exit 2 ;;
    esac
done

cd "$web"

if [[ "${PHOTOVAULT_SKIP_NPM_CI:-}" != 1 ]]; then
    log "npm ci"
    npm ci --no-audit --no-fund
fi

# wasm-pack downloads binaryen (wasm-opt) itself, but its downloader ignores
# the system CA store, which fails behind TLS-intercepting proxies. It uses a
# wasm-opt found on PATH instead, so fetch the release it expects with curl.
ensure_wasm_opt() {
    command -v wasm-opt >/dev/null 2>&1 && return 0
    local ver=version_117
    local cache="${XDG_CACHE_HOME:-$HOME/.cache}/photovault-build"
    local dir="$cache/binaryen-$ver"
    if [[ ! -x "$dir/bin/wasm-opt" ]]; then
        local arch
        case "$(uname -m)" in
            x86_64) arch=x86_64 ;;
            aarch64 | arm64) arch=aarch64 ;;
            *) echo "no binaryen download for $(uname -m); set PHOTOVAULT_WASM_NO_OPT=1" >&2; return 1 ;;
        esac
        local os=linux
        [[ "$(uname -s)" == Darwin ]] && os=macos
        mkdir -p "$cache"
        curl -fsSL --retry 5 --retry-all-errors -o "$cache/binaryen.tgz" \
            "https://github.com/WebAssembly/binaryen/releases/download/$ver/binaryen-$ver-$arch-$os.tar.gz"
        tar -xzf "$cache/binaryen.tgz" -C "$cache"
        rm -f "$cache/binaryen.tgz"
    fi
    export PATH="$dir/bin:$PATH"
}

# Same arguments as the workspaces' own "build" scripts
# (packages/wasm/{prelogin,photos,cast}/package.json), plus --no-opt.
build_wasm_no_opt() {
    local name="$1"
    local crate="$repo/rust/bindings/wasm/$name"
    local out="$web/packages/wasm/$name/pkg"
    if [[ "$name" == cast ]]; then
        (cd "$crate" && wasm-pack build . --no-opt --target bundler \
            --out-dir "$out" --no-pack -- -Z build-std=panic_abort,std)
    else
        wasm-pack build "$crate" --no-opt --target bundler --out-dir "$out" --no-pack
    fi
}

if [[ "${PHOTOVAULT_SKIP_WASM:-}" != 1 ]]; then
    wasm=(prelogin)
    [[ " $apps " == *" photos "* ]] && wasm+=(photos)
    [[ " $apps " == *" cast "* ]] && wasm+=(cast)
    if [[ "${PHOTOVAULT_WASM_NO_OPT:-}" == 1 ]]; then
        for w in "${wasm[@]}"; do
            log "wasm: $w (no wasm-opt)"
            build_wasm_no_opt "$w"
        done
    else
        ensure_wasm_opt
        for w in "${wasm[@]}"; do
            log "wasm: $w"
            npm run "build:$w-wasm"
        done
    fi
fi

if [[ " $apps " == *" photos "* && "${PHOTOVAULT_SKIP_ASSETS:-}" != 1 ]]; then
    log "static data files (/static-assets/)"
    # Not fatal: without them only places, city search and in-browser video
    # processing degrade (see the vendor script).
    sh "$here/photovault-vendor-assets.sh" ${PHOTOVAULT_ASSETS_SOURCE:+"$PHOTOVAULT_ASSETS_SOURCE"} ||
        echo "warning: static data files not fetched; continuing without them" >&2
fi

for app in $apps; do
    log "next build: $app (NEXT_PUBLIC_ENTE_ENDPOINT='${NEXT_PUBLIC_ENTE_ENDPOINT}')"
    rm -rf "apps/$app/out"
    npm exec --workspace "$app" -- next build --webpack
    test -f "apps/$app/out/index.html"
done

# The sub-apps reference a few files by absolute path (/fonts/*.woff2 in the
# albums CSS, /images/*.png in accounts). Under one origin those resolve
# against the Photos export, so copy the ones Photos lacks there (never
# overwriting Photos' own files).
if [[ -d apps/photos/out ]]; then
    for app in albums cast accounts; do
        [[ -d "apps/$app/public" ]] || continue
        (cd "apps/$app/public" && find fonts images -type f 2>/dev/null || true) |
            while IFS= read -r f; do
                if [[ ! -e "apps/photos/out/$f" ]]; then
                    mkdir -p "apps/photos/out/$(dirname "$f")"
                    cp "apps/$app/public/$f" "apps/photos/out/$f"
                fi
            done
    done
fi

if [[ -n "${PHOTOVAULT_WEB_OUT:-}" ]]; then
    log "assembling $PHOTOVAULT_WEB_OUT"
    rm -rf "$PHOTOVAULT_WEB_OUT"
    mkdir -p "$PHOTOVAULT_WEB_OUT"
    cp -a apps/photos/out/. "$PHOTOVAULT_WEB_OUT/"
    for app in albums cast accounts; do
        if [[ -f "apps/$app/out/index.html" ]]; then
            mkdir -p "$PHOTOVAULT_WEB_OUT/$app"
            cp -a "apps/$app/out/." "$PHOTOVAULT_WEB_OUT/$app/"
        fi
    done
fi

log "done"
for app in $apps; do echo "$web/apps/$app/out"; done
