#!/usr/bin/env bash
# Run the S3 backend integration tests against a throwaway Garage container.
#
#   scripts/test-s3-garage.sh            # start Garage, run tests, remove Garage
#   KEEP=1 scripts/test-s3-garage.sh     # leave the container running afterwards
#
# Garage is a TEST dependency only. Override the image with GARAGE_IMAGE
# (e.g. a registry mirror) and the host port with GARAGE_PORT.
set -euo pipefail

image="${GARAGE_IMAGE:-dxflrs/garage:v2.1.0}"
port="${GARAGE_PORT:-39000}"
name="photovault-test-garage-$$"
# Fixed test credentials (Garage's formats: GK + 24 hex, 64 hex).
key_id="GK0123456789abcdef01234567"
secret="0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
bucket="pv-test"
repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
workdir="$(mktemp -d)"

cleanup() {
    if [[ "${KEEP:-}" != "1" ]]; then
        docker rm -f "$name" >/dev/null 2>&1 || true
    fi
    rm -rf -- "$workdir"
}
trap cleanup EXIT

cat >"$workdir/garage.toml" <<EOF
metadata_dir = "/var/lib/garage/meta"
data_dir = "/var/lib/garage/data"
db_engine = "sqlite"
replication_factor = 1
rpc_bind_addr = "0.0.0.0:3901"
rpc_public_addr = "127.0.0.1:3901"
rpc_secret = "$(head -c 32 /dev/urandom | od -An -tx1 | tr -d ' \n')"

[s3_api]
s3_region = "garage"
api_bind_addr = "0.0.0.0:3900"
root_domain = ".s3.garage.localhost"
EOF

docker run -d --name "$name" -p "127.0.0.1:${port}:3900" \
    -v "$workdir/garage.toml:/etc/garage.toml:ro" "$image" >/dev/null

g() { docker exec "$name" /garage "$@"; }

for _ in $(seq 1 50); do
    if g status >/dev/null 2>&1; then break; fi
    sleep 0.2
done
node_id="$(g status 2>/dev/null | awk '/^[0-9a-f]{16}/ {print $1; exit}')"
[[ -n "$node_id" ]] || { echo "garage did not start" >&2; docker logs "$name" >&2; exit 1; }
g layout assign -z dc1 -c 1G "$node_id" >/dev/null
g layout apply --version 1 >/dev/null
g key import --yes -n pv-test "$key_id" "$secret" >/dev/null
g bucket create "$bucket" >/dev/null
g bucket allow --read --write --owner "$bucket" --key "$key_id" >/dev/null

export PHOTOVAULT_TEST_S3_ENDPOINT="http://127.0.0.1:${port}"
export PHOTOVAULT_TEST_S3_REGION="garage"
export PHOTOVAULT_TEST_S3_BUCKET="$bucket"
export PHOTOVAULT_TEST_S3_ACCESS_KEY="$key_id"
export PHOTOVAULT_TEST_S3_SECRET_KEY="$secret"

cd "$repo_root/server"
cargo test --test storage_s3 -- --nocapture
