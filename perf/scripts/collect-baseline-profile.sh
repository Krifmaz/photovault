#!/usr/bin/env bash
# Generate the Baseline Profile and startup profile for Photovault Native and copy
# them into the native app's sources, where the native track checks them in:
#
#   native/android/app/src/main/baseline-prof.txt                    (Baseline Profile)
#   native/android/app/src/main/baselineProfiles/startup-prof.txt    (startup profile,
#                                                                     DEX layout)
#
# Usage:
#   perf/scripts/collect-baseline-profile.sh [options]
#
# Options:
#   -s SERIAL          adb serial (default: $ANDROID_SERIAL or the only device)
#   --apk PATH         install this release build of the native app first
#   --test-apk PATH    baselineprofile test APK (default: perf/android build output)
#   --package PKG      target package (default app.photovault.native)
#   --app-dir DIR      the native app module (default native/android/app)
#   --emulator         allow an emulator (suppresses the EMULATOR error)
#
# Device: Android 13+ (API 33), or rooted API 28+. Use a release (non-debuggable)
# build, signed in or in local-only mode with photos, like the benchmarks.
set -euo pipefail

# shellcheck source-path=SCRIPTDIR source=lib.sh
source "$(dirname "${BASH_SOURCE[0]}")/lib.sh"

APK=""; TEST_APK=""; PKG="$NATIVE_PKG_DEFAULT"; APP_DIR="$REPO_ROOT/native/android/app"; EMULATOR=false

usage() { sed -n '2,/^set -euo/p' "$0" | sed '$d; s/^# \{0,1\}//'; exit "${1:-0}"; }

while [[ $# -gt 0 ]]; do
    case "$1" in
        -s) ADB_SERIAL="$2"; shift 2 ;;
        --apk) APK="$2"; shift 2 ;;
        --test-apk) TEST_APK="$2"; shift 2 ;;
        --package) PKG="$2"; shift 2 ;;
        --app-dir) APP_DIR="$2"; shift 2 ;;
        --emulator) EMULATOR=true; shift ;;
        -h|--help) usage 0 ;;
        *) warn "unknown option $1"; usage 2 ;;
    esac
done

require_device
SDK="$(sdk_int)"
if [[ "$SDK" -lt 33 ]]; then
    [[ "$(sh_ id -u 2>/dev/null)" == 0 ]] || sh_ su -c id >/dev/null 2>&1 ||
        die "API $SDK: Baseline Profile generation needs Android 13+ (API 33) or a rooted device"
fi

[[ -n "$APK" ]] && install_apk "$APK"
is_installed "$PKG" || die "$PKG is not installed; pass --apk"
is_debuggable "$PKG" && warn "$PKG is debuggable; generate profiles from a release build"

if [[ -z "$TEST_APK" ]]; then
    TEST_APK="$(first_existing \
        "$PERF_DIR/android/baselineprofile/build/outputs/apk/release/baselineprofile-release.apk" \
        "$PERF_DIR/android/baselineprofile/build/outputs/apk/debug/baselineprofile-debug.apk")" ||
        die "no baselineprofile APK; build it with: gradle -p perf/android :baselineprofile:assembleRelease (or pass --test-apk)"
fi
install_apk "$TEST_APK"

MEDIA="$(media_dir "$BASELINE_PKG")"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
sh_ rm -rf "$MEDIA" >/dev/null || true

args=(-e class "$BASELINE_PKG.BaselineProfileGenerator" -e targetPackage "$PKG" -e appKind native)
$EMULATOR && args+=(-e androidx.benchmark.suppressErrors EMULATOR)
run_instrumentation "$BASELINE_PKG" "$WORK/instrumentation.log" "${args[@]}" ||
    { tail -n 30 "$WORK/instrumentation.log" >&2; die "generator failed (log above)"; }

adb_ pull "$MEDIA" "$WORK/" >/dev/null || die "could not pull $MEDIA"

# BaselineProfileRule writes <Class>_<method>-baseline-prof.txt and -startup-prof.txt
# (some versions also write timestamped copies with the same content).
pick() {
    local exact="$1" pattern="$2" found
    found="$(find "$WORK" -type f -name "$exact" | head -n 1)"
    [[ -n "$found" ]] || found="$(find "$WORK" -type f -name "$pattern" | sort | tail -n 1)"
    printf '%s' "$found"
}
BASELINE="$(pick 'BaselineProfileGenerator_generate-baseline-prof.txt' '*-baseline-prof*.txt')"
STARTUP="$(pick 'BaselineProfileGenerator_generate-startup-prof.txt' '*-startup-prof*.txt')"
[[ -n "$BASELINE" ]] || die "no *-baseline-prof.txt in $MEDIA; see $WORK/instrumentation.log"

mkdir -p "$APP_DIR/src/main/baselineProfiles"
cp "$BASELINE" "$APP_DIR/src/main/baseline-prof.txt"
log "Wrote $APP_DIR/src/main/baseline-prof.txt ($(wc -l <"$BASELINE" | tr -d ' ') rules)"
if [[ -n "$STARTUP" ]]; then
    cp "$STARTUP" "$APP_DIR/src/main/baselineProfiles/startup-prof.txt"
    log "Wrote $APP_DIR/src/main/baselineProfiles/startup-prof.txt ($(wc -l <"$STARTUP" | tr -d ' ') rules)"
else
    warn "no startup profile was produced"
fi
log "Commit these in the native track. The native app needs androidx.profileinstaller (Compose brings it) so sideloaded installs use the profile."
