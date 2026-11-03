#!/usr/bin/env bash
# End-to-end tests: drive the UNMODIFIED upstream Ente Photos web app (built by
# build-web.sh at the commit in UPSTREAM_COMMIT) against a freshly started
# photovault server, with Playwright.
#
# Usage: tests/e2e/run.sh [test-name ...]     (no names: run every test)
#
# Environment:
#   PHOTOVAULT_E2E_CACHE         build cache (default ~/.cache/photovault-e2e)
#   PHOTOVAULT_E2E_EXTERNAL_API  1 = do not build/start photovault; use whatever
#                                already listens on the API port (e.g. museum,
#                                to validate the harness itself)
#   PHOTOVAULT_E2E_EXTRA_CONFIG  path of a TOML snippet appended to the
#                                generated server config
#   E2E_HEADED                   1 = show the browser (needs a display)
#
# Ports are fixed because the web app bakes its API origin in at build time:
# API 18080, photos app 13000, public albums app 13002.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo="$(cd "$here/../.." && pwd)"
api_port=18080
web_port=13000
albums_port=13002
api="http://localhost:$api_port"
artifacts="$here/artifacts"

rm -rf "$artifacts"
mkdir -p "$artifacts"

tmp="$(mktemp -d "${TMPDIR:-/tmp}/photovault-e2e.XXXXXX")"
pids=()
cleanup() {
    for pid in "${pids[@]}"; do
        kill "$pid" 2>/dev/null || true
    done
    wait 2>/dev/null || true
    rm -rf "$tmp"
}
trap cleanup EXIT

port_free() {
    ! (exec 3<>"/dev/tcp/127.0.0.1/$1") 2>/dev/null
}

wait_http() {
    local url="$1" tries=0
    until curl -fsS -o /dev/null "$url" 2>/dev/null; do
        tries=$((tries + 1))
        if ((tries > 120)); then
            echo "timed out waiting for $url" >&2
            return 1
        fi
        sleep 0.5
    done
}

for p in "$web_port" "$albums_port"; do
    port_free "$p" || { echo "port $p is in use" >&2; exit 1; }
done

echo "== building web app (cached after the first run)"
web_out="$(PHOTOVAULT_E2E_API="$api" "$here/build-web.sh" | tail -n 1)"

if [[ "${PHOTOVAULT_E2E_EXTERNAL_API:-0}" != 1 ]]; then
    port_free "$api_port" || { echo "port $api_port is in use" >&2; exit 1; }
    echo "== building photovault (debug)"
    (cd "$repo" && cargo build -p photovault-server --bin photovault)
    bin="$repo/target/debug/photovault"

    config="$tmp/photovault.toml"
    {
        printf 'listen = "127.0.0.1:%s"\n' "$api_port"
        printf 'public_url = "%s"\n' "$api"
        printf 'database_url = "sqlite://%s/db.sqlite?mode=rwc"\n' "$tmp"
        printf 'log_format = "pretty"\n'
        printf 'allow_signups = true\n'
        printf '\n[storage]\nkind = "local"\npath = "%s/objects"\n' "$tmp"
        printf '\n[hardcoded_ott]\n'
        printf '"e2e@example.com" = "123456"\n'
        printf '"e2e2@example.com" = "123456"\n'
        printf '"e2e3@example.com" = "123456"\n'
        if [[ -n "${PHOTOVAULT_E2E_EXTRA_CONFIG:-}" ]]; then
            printf '\n'
            cat "$PHOTOVAULT_E2E_EXTRA_CONFIG"
        fi
    } > "$config"
    cp "$config" "$artifacts/photovault.toml"

    echo "== starting photovault on $api"
    RUST_LOG="${RUST_LOG:-info}" "$bin" --config "$config" serve > "$artifacts/server.log" 2>&1 &
    pids+=("$!")
    wait_http "$api/healthz"
fi

echo "== serving web apps"
node "$here/lib/static-server.mjs" "$web_out/photos" "$web_port" > "$artifacts/static-photos.log" 2>&1 &
pids+=("$!")
node "$here/lib/static-server.mjs" "$web_out/albums" "$albums_port" > "$artifacts/static-albums.log" 2>&1 &
pids+=("$!")
wait_http "http://localhost:$web_port/"
wait_http "http://localhost:$albums_port/"

echo "== running Playwright"
status=0
E2E_API_URL="$api" \
    E2E_WEB_URL="http://localhost:$web_port" \
    E2E_ALBUMS_URL="http://localhost:$albums_port" \
    E2E_ARTIFACTS="$artifacts" \
    E2E_TMP="$tmp" \
    node "$here/runner.mjs" "$@" || status=$?

echo "== results: $artifacts/results.json (screenshots and logs for failures in $artifacts/)"
exit "$status"
