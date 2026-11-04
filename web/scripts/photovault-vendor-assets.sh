#!/bin/sh
# PHOTOVAULT: strip (new file).
#
# Kept for callers that use the shell name; the work (and the list of files and
# why they are needed) is in photovault-vendor-assets.mjs, which also runs on
# Windows (desktop builds).
#
# Usage: scripts/photovault-vendor-assets.sh [SOURCE]

set -eu

exec node "$(dirname "$0")/photovault-vendor-assets.mjs" "$@"
