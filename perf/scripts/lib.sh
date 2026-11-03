# shellcheck shell=bash
# shellcheck disable=SC2034  # constants are used by the scripts that source this file
# Shared helpers for perf/scripts/*.sh. Source it; do not run it.

PERF_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
REPO_ROOT="$(cd "$PERF_DIR/.." && pwd)"
RUNNER="androidx.test.runner.AndroidJUnitRunner"
MACRO_PKG="app.photovault.perf.macrobenchmark"
BASELINE_PKG="app.photovault.perf.baselineprofile"
NATIVE_PKG_DEFAULT="app.photovault.native"
FLUTTER_PKG_DEFAULT="app.photovault"

ADB_SERIAL="${ANDROID_SERIAL:-}"

log() { printf '\033[1m==>\033[0m %s\n' "$*" >&2; }
warn() { printf '\033[33mwarning:\033[0m %s\n' "$*" >&2; }
die() { printf '\033[31merror:\033[0m %s\n' "$*" >&2; exit 1; }

adb_() {
    if [[ -n "$ADB_SERIAL" ]]; then adb -s "$ADB_SERIAL" "$@"; else adb "$@"; fi
}

# adb shell with CR stripped from output.
sh_() { adb_ shell "$@" | tr -d '\r'; }

require_device() {
    command -v adb >/dev/null || die "adb not found on PATH (Android SDK platform-tools)"
    local devices count
    devices="$(adb devices | awk 'NR > 1 && $2 == "device" { print $1 }')"
    count="$(printf '%s\n' "$devices" | grep -c . || true)"
    if [[ -z "$ADB_SERIAL" ]]; then
        [[ "$count" -ge 1 ]] || die "no device connected (adb devices)"
        [[ "$count" -eq 1 ]] || die "several devices connected; pass -s SERIAL or set ANDROID_SERIAL"
        ADB_SERIAL="$devices"
    else
        printf '%s\n' "$devices" | grep -qx "$ADB_SERIAL" || die "device $ADB_SERIAL not connected"
    fi
}

sdk_int() { sh_ getprop ro.build.version.sdk; }

is_installed() { [[ -n "$(sh_ pm path "$1" 2>/dev/null)" ]]; }

is_debuggable() { sh_ dumpsys package "$1" | grep -qE 'pkgFlags=\[[^]]*DEBUGGABLE'; }

app_version() { sh_ dumpsys package "$1" | sed -n 's/^ *versionName=//p' | head -n 1; }

install_apk() {
    local apk="$1"
    [[ -f "$apk" ]] || die "APK not found: $apk"
    log "Installing $(basename "$apk")"
    if ! adb_ install -r -d -g "$apk"; then
        die "install failed for $apk. If it is INSTALL_FAILED_UPDATE_INCOMPATIBLE the installed build has a different signing key; uninstall it by hand (this wipes its sign-in and library state)."
    fi
}

# Pick the first existing file among the arguments.
first_existing() {
    local f
    for f in "$@"; do [[ -f "$f" ]] && { printf '%s\n' "$f"; return 0; }; done
    return 1
}

# run_instrumentation TEST_PKG LOGFILE [am instrument -e args...]
# Returns non-zero when the run failed.
run_instrumentation() {
    local test_pkg="$1" logfile="$2"; shift 2
    local extra=()
    if [[ "$(sdk_int)" -ge 30 ]]; then extra+=(--no-isolated-storage); fi
    log "am instrument $test_pkg $*"
    adb_ shell am instrument -w -r ${extra[@]+"${extra[@]}"} "$@" \
        -e androidx.benchmark.output.enable true \
        "$test_pkg/$RUNNER" | tr -d '\r' | tee "$logfile"
    if grep -qE 'FAILURES!!!|INSTRUMENTATION_FAILED|INSTRUMENTATION_CODE: 0|Process crashed|shortMsg=' "$logfile"; then
        return 1
    fi
    # Activity.RESULT_OK (-1) is what a completed instrumentation run reports.
    grep -q 'INSTRUMENTATION_CODE: -1' "$logfile"
}

# Media directory where Macrobenchmark / BaselineProfileRule write their outputs.
media_dir() { printf '/sdcard/Android/media/%s' "$1"; }
