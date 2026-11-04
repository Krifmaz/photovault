#!/usr/bin/env bash
# PHOTOVAULT: before/after screenshots of the Ente screens Photovault changes.
#
#   mobile/apps/photos/scripts/photovault/render_before_after.sh <out-dir> [upstream-commit]
#
# "After" renders test/photovault/before_after/ (and the after-only screens
# in test/photovault/screenshots_test.dart) in this tree. "Before" renders
# the same test in a temporary detached worktree of the pristine Ente import
# (default 3b2cede), with tree_env_upstream.dart as tree_env.dart. Writes
# <screen>-<mode>-before.png / -after.png into <out-dir>, then removes the
# worktree and its Rust build output. Needs Flutter and cargo (for
# `cargo codegen frb photos`); about 3 GB of disk while it runs.
set -euo pipefail

out=$(realpath -m "${1:?usage: $0 <out-dir> [upstream-commit]}")
commit=${2:-3b2cede}
app=$(cd "$(dirname "$0")/../.." && pwd)
repo=$(git -C "$app" rev-parse --show-toplevel)
work=$(mktemp -d)
upstream="$work/upstream"
trap 'git -C "$repo" worktree remove --force "$upstream" 2>/dev/null || true; rm -rf "$work"' EXIT

mkdir -p "$out"

echo "== after ($app)"
(cd "$app" && PHOTOVAULT_SCREENSHOT_DIR="$work/after" \
    flutter test test/photovault/before_after/before_after_test.dart)
(cd "$app" && PHOTOVAULT_SCREENSHOT_DIR="$work/after-only" \
    flutter test test/photovault/screenshots_test.dart)

echo "== before ($commit)"
git -C "$repo" worktree add --detach "$upstream" "$commit"
up_app="$upstream/mobile/apps/photos"
mkdir -p "$up_app/test/photovault/before_after"
cp "$app/test/photovault/before_after/before_after_test.dart" \
    "$up_app/test/photovault/before_after/"
cp "$app/test/photovault/before_after/tree_env_upstream.dart" \
    "$up_app/test/photovault/before_after/tree_env.dart"
(cd "$up_app" && flutter pub get)
(cd "$upstream/rust" && CARGO_TARGET_DIR="$work/target" cargo codegen frb photos)
rm -rf "$work/target"
# Ente animates its duck with Rive (rive_native, FFI). Its prebuilt host
# library comes from Rive's CDN; build it instead from the C++ sources
# shipped in the pub package, so the "before" shots show the duck.
rive_pkg=$(python3 -I -c 'import json,sys
for p in json.load(open(sys.argv[1]))["packages"]:
    if p["name"] == "rive_native": print(p["rootUri"].removeprefix("file://"))' \
    "$upstream/mobile/.dart_tool/package_config.json")
cp -r "$rive_pkg" "$work/rive_native"
cp "$work/rive_native/runtime/renderer/src/shaders/Makefile.rive" \
    "$work/rive_native/runtime/renderer/src/shaders/Makefile"
(cd "$work/rive_native/native" && ./build.sh shared flutter-runtime > "$work/rive-build.log" 2>&1)
mkdir -p "$up_app/build/rive_native/linux/bin/lib/debug_shared"
cp "$work/rive_native/linux/bin/lib/debug_shared/librive_native.so" \
    "$up_app/build/rive_native/linux/bin/lib/debug_shared/"
rm -rf "$work/rive_native"
# The upstream run reports a failure from Ente's billing plugin (no Play
# Billing off-device) after the Family shot is saved; the images are fine.
(cd "$up_app" && PHOTOVAULT_SCREENSHOT_DIR="$work/before" \
    flutter test test/photovault/before_after/before_after_test.dart || true)

for f in "$work"/before/*.png; do
    cp "$f" "$out/$(basename "$f" .png)-before.png"
done
# after/ last: its about/settings shots replace the excerpts in after-only/.
for f in "$work"/after-only/*.png "$work"/after/*.png; do
    cp "$f" "$out/$(basename "$f" .png)-after.png"
done
echo "wrote $(ls "$out" | wc -l) files to $out"
