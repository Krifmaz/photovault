#!/usr/bin/env bash
# Measure both Photovault APKs on a real high-refresh phone over adb.
#
#   1. reads the display's supported and current refresh rates
#      (dumpsys display / dumpsys SurfaceFlinger)
#   2. forces the peak refresh rate (settings peak_refresh_rate / min_refresh_rate),
#      restored on exit unless --keep-refresh
#   3. installs the given release APKs and the macrobenchmark test APK
#   4. runs the macrobenchmarks with -e targetPackage for each app
#   5. for the native app: dumpsys gfxinfo reset, scripted adb swipes on the timeline,
#      dumpsys gfxinfo framestats -> janky-frame %, p90/p99 frame times
#   6. writes perf/results/<date>-<device>.md (raw data under perf/results/raw/)
#
# Usage:
#   perf/scripts/device-120hz.sh [options]
#
# Options:
#   -s SERIAL              adb serial (default: $ANDROID_SERIAL or the only device)
#   --native-apk PATH      install this Photovault Native APK first
#   --flutter-apk PATH     install this Photovault (Flutter) APK first
#   --test-apk PATH        macrobenchmark test APK (default: perf/android build output)
#   --apps LIST            comma list of native,flutter (default: both, skipping any
#                          that are not installed)
#   --native-package PKG   default app.photovault.native
#   --flutter-package PKG  default app.photovault
#   --iterations N         macrobenchmark iterations (default 10)
#   --compilation MODE     none | partial | full (default partial)
#   --flutter-trace        pass flutterTrace=true (needs a --profile Flutter build)
#   --swipes N             adb swipes for the gfxinfo pass (default 20)
#   --no-macro             skip macrobenchmarks
#   --no-gfxinfo           skip the gfxinfo pass
#   --smoke                app not signed in (fresh emulator): startup to first frame
#                          only, scroll/viewer benchmarks and gfxinfo skipped
#   --emulator             emulator run: no refresh forcing, suppress the
#                          EMULATOR (and DEBUGGABLE) benchmark errors, never enforce
#   --enforce              exit 1 when a pass criterion fails (default on a phone)
#   --no-enforce           report only
#   --keep-refresh         leave the forced refresh-rate settings in place
#   --out DIR              results directory (default perf/results)
#
# Requires: adb, python3. Device: signed in (or local-only) with a library large
# enough to fling through; see perf/README.md "Device preparation".
set -euo pipefail

# shellcheck source-path=SCRIPTDIR source=lib.sh
source "$(dirname "${BASH_SOURCE[0]}")/lib.sh"

NATIVE_APK=""; FLUTTER_APK=""; TEST_APK=""; APPS="native,flutter"
NATIVE_PKG="$NATIVE_PKG_DEFAULT"; FLUTTER_PKG="$FLUTTER_PKG_DEFAULT"
ITERATIONS=10; COMPILATION=partial; FLUTTER_TRACE=false; SWIPES=20
RUN_MACRO=true; RUN_GFXINFO=true; SMOKE=false; EMULATOR=false; ENFORCE=""; KEEP_REFRESH=false
OUT_DIR="$PERF_DIR/results"

usage() { sed -n '2,/^set -euo/p' "$0" | sed '$d; s/^# \{0,1\}//'; exit "${1:-0}"; }

while [[ $# -gt 0 ]]; do
    case "$1" in
        -s) ADB_SERIAL="$2"; shift 2 ;;
        --native-apk) NATIVE_APK="$2"; shift 2 ;;
        --flutter-apk) FLUTTER_APK="$2"; shift 2 ;;
        --test-apk) TEST_APK="$2"; shift 2 ;;
        --apps) APPS="$2"; shift 2 ;;
        --native-package) NATIVE_PKG="$2"; shift 2 ;;
        --flutter-package) FLUTTER_PKG="$2"; shift 2 ;;
        --iterations) ITERATIONS="$2"; shift 2 ;;
        --compilation) COMPILATION="$2"; shift 2 ;;
        --flutter-trace) FLUTTER_TRACE=true; shift ;;
        --swipes) SWIPES="$2"; shift 2 ;;
        --no-macro) RUN_MACRO=false; shift ;;
        --no-gfxinfo) RUN_GFXINFO=false; shift ;;
        --smoke) SMOKE=true; RUN_GFXINFO=false; shift ;;
        --emulator) EMULATOR=true; shift ;;
        --enforce) ENFORCE=true; shift ;;
        --no-enforce) ENFORCE=false; shift ;;
        --keep-refresh) KEEP_REFRESH=true; shift ;;
        --out) OUT_DIR="$2"; shift 2 ;;
        -h|--help) usage 0 ;;
        *) warn "unknown option $1"; usage 2 ;;
    esac
done
[[ -n "$ENFORCE" ]] || { if $EMULATOR; then ENFORCE=false; else ENFORCE=true; fi; }
$EMULATOR && ENFORCE=false

command -v python3 >/dev/null || die "python3 is required for the report"
require_device
REPORT_PY="$PERF_DIR/scripts/perf_report.py"

# ------------------------------------------------------------------ run setup
DATE="$(date +%Y-%m-%d)"
MODEL="$(sh_ getprop ro.product.model)"
SLUG="$(printf '%s' "$MODEL" | tr '[:upper:]' '[:lower:]' | tr -cs 'a-z0-9' '-' | sed 's/^-//; s/-$//')"
[[ -n "$SLUG" ]] || SLUG="device"
$EMULATOR && SLUG="emulator-$SLUG"
RAW="$OUT_DIR/raw/$DATE-$SLUG"
REPORT="$OUT_DIR/$DATE-$SLUG.md"
META="$RAW/meta.json"
mkdir -p "$RAW"
rm -f "$META"

meta() { python3 "$REPORT_PY" meta "$META" "$@"; }
meta set date "$DATE"
meta set serial "$ADB_SERIAL"
meta set model "$MODEL"
meta set manufacturer "$(sh_ getprop ro.product.manufacturer)"
meta set release "$(sh_ getprop ro.build.version.release)"
meta set sdk "$(sdk_int)"
meta set fingerprint "$(sh_ getprop ro.build.fingerprint)"
meta set emulator "$EMULATOR"
log "Device: $MODEL (API $(sdk_int)), results -> $REPORT"

# --------------------------------------------------------------- refresh rate
# refresh_snapshot NAME: dump display + SurfaceFlinger, store parsed JSON under NAME.
refresh_snapshot() {
    local name="$1"
    sh_ dumpsys display >"$RAW/display-$name.txt" || true
    sh_ dumpsys SurfaceFlinger >"$RAW/surfaceflinger-$name.txt" || true
    python3 "$REPORT_PY" refresh --display "$RAW/display-$name.txt" \
        --sf "$RAW/surfaceflinger-$name.txt" >"$RAW/refresh-$name.json"
    meta setjson "$name" "$RAW/refresh-$name.json"
    python3 - "$RAW/refresh-$name.json" "$name" <<'PY'
import json, sys
r = json.load(open(sys.argv[1]))
print(f"    refresh [{sys.argv[2]}]: current={r['currentHz']} Hz ({r['currentSource']}), "
      f"peak={r['maxHz']} Hz, supported={r['supportedHz']}", file=sys.stderr)
PY
}

json_field() { python3 -c 'import json,sys; v=json.load(open(sys.argv[1])).get(sys.argv[2]); print("" if v is None else v)' "$1" "$2"; }

PREV_PEAK=""; PREV_MIN=""; FORCED=false
# shellcheck disable=SC2317,SC2329  # invoked by the EXIT trap (SC2317 on older shellcheck, SC2329 on newer)
restore_refresh() {
    if $FORCED && ! $KEEP_REFRESH; then
        log "Restoring refresh-rate settings"
        if [[ -z "$PREV_PEAK" || "$PREV_PEAK" == "null" ]]; then
            sh_ settings delete system peak_refresh_rate >/dev/null || true
        else
            sh_ settings put system peak_refresh_rate "$PREV_PEAK" || true
        fi
        if [[ -z "$PREV_MIN" || "$PREV_MIN" == "null" ]]; then
            sh_ settings delete system min_refresh_rate >/dev/null || true
        else
            sh_ settings put system min_refresh_rate "$PREV_MIN" || true
        fi
    fi
}
trap restore_refresh EXIT

log "Checking refresh rate"
refresh_snapshot refreshBefore
PEAK_HZ="$(json_field "$RAW/refresh-refreshBefore.json" maxHz)"
if $EMULATOR; then
    log "Emulator: not forcing a refresh rate"
elif [[ -z "$PEAK_HZ" ]]; then
    warn "could not read the supported refresh rates; not forcing one"
else
    awk -v hz="$PEAK_HZ" 'BEGIN { exit !(hz + 0 >= 119) }' ||
        warn "peak refresh rate is $PEAK_HZ Hz, below 120 Hz: the 8.33 ms budget is stricter than this display needs"
    PREV_PEAK="$(sh_ settings get system peak_refresh_rate)"
    PREV_MIN="$(sh_ settings get system min_refresh_rate)"
    log "Forcing peak refresh rate $PEAK_HZ Hz (was peak=$PREV_PEAK min=$PREV_MIN)"
    sh_ settings put system peak_refresh_rate "$PEAK_HZ"
    sh_ settings put system min_refresh_rate "$PEAK_HZ"
    FORCED=true
    sleep 1
    refresh_snapshot refreshForced
fi

# ------------------------------------------------------------------- installs
[[ -n "$NATIVE_APK" ]] && install_apk "$NATIVE_APK"
[[ -n "$FLUTTER_APK" ]] && install_apk "$FLUTTER_APK"

declare -a TARGET_KINDS=()
for kind in ${APPS//,/ }; do
    case "$kind" in
        native) pkg="$NATIVE_PKG" ;;
        flutter) pkg="$FLUTTER_PKG" ;;
        *) die "unknown app kind '$kind' in --apps" ;;
    esac
    if ! is_installed "$pkg"; then
        warn "$pkg is not installed; skipping $kind"
        continue
    fi
    debuggable=false
    if is_debuggable "$pkg"; then
        debuggable=true
        warn "$pkg is debuggable: numbers from a debug build are meaningless; install a release build"
    fi
    TARGET_KINDS+=("$kind")
    tmp="$RAW/app-$kind.json"
    python3 -c 'import json,sys; json.dump({"kind":sys.argv[1],"package":sys.argv[2],"version":sys.argv[3],"debuggable":sys.argv[4]=="true"}, open(sys.argv[5],"w"))' \
        "$kind" "$pkg" "$(app_version "$pkg")" "$debuggable" "$tmp"
    meta appendjson apps "$tmp"
done
[[ ${#TARGET_KINDS[@]} -gt 0 ]] || die "neither app is installed; pass --native-apk / --flutter-apk"

pkg_for() { if [[ "$1" == native ]]; then echo "$NATIVE_PKG"; else echo "$FLUTTER_PKG"; fi; }

# ------------------------------------------------------------ macrobenchmarks
SUMMARY_ARGS=()
if $RUN_MACRO; then
    if [[ -z "$TEST_APK" ]]; then
        TEST_APK="$(first_existing \
            "$PERF_DIR/android/macrobenchmark/build/outputs/apk/release/macrobenchmark-release.apk" \
            "$PERF_DIR/android/macrobenchmark/build/outputs/apk/debug/macrobenchmark-debug.apk")" ||
            die "no macrobenchmark APK; build it with: gradle -p perf/android :macrobenchmark:assembleRelease (or pass --test-apk)"
    fi
    install_apk "$TEST_APK"

    SUPPRESS=""
    if $EMULATOR; then SUPPRESS="EMULATOR,DEBUGGABLE"; fi

    for kind in ${TARGET_KINDS[@]+"${TARGET_KINDS[@]}"}; do
        pkg="$(pkg_for "$kind")"
        out="$RAW/macro-$kind"
        mkdir -p "$out"
        sh_ rm -rf "$(media_dir "$MACRO_PKG")" >/dev/null || true
        args=(-e package "$MACRO_PKG" -e targetPackage "$pkg" -e appKind "$kind"
            -e iterations "$ITERATIONS" -e compilation "$COMPILATION")
        [[ "$kind" == flutter ]] && $FLUTTER_TRACE && args+=(-e flutterTrace true)
        $SMOKE && args+=(-e requireTimeline false)
        [[ -n "$SUPPRESS" ]] && args+=(-e androidx.benchmark.suppressErrors "$SUPPRESS")
        log "Macrobenchmarks for $pkg"
        if ! run_instrumentation "$MACRO_PKG" "$out/instrumentation.log" "${args[@]}"; then
            warn "macrobenchmark run for $pkg failed; see $out/instrumentation.log"
            meta append failures "macrobenchmark $kind ($pkg): see raw/$DATE-$SLUG/macro-$kind/instrumentation.log"
        fi
        adb_ pull "$(media_dir "$MACRO_PKG")" "$out/" >/dev/null 2>&1 ||
            warn "nothing to pull from $(media_dir "$MACRO_PKG")"
        SUMMARY_ARGS+=(--macro "$kind=$out")
    done
fi

# --------------------------------------------------- gfxinfo pass (native only)
run_gfxinfo() {
    local pkg="$1" w h x y_top y_bottom i dumps=()
    log "gfxinfo framestats pass for $pkg ($SWIPES swipes)"
    sh_ am force-stop "$pkg"
    sh_ monkey -p "$pkg" -c android.intent.category.LAUNCHER 1 >/dev/null
    sleep 4
    refresh_snapshot refreshDuring
    local size
    size="$(sh_ wm size | awk -F': ' '/Override size/ { o = $2 } /Physical size/ { p = $2 } END { print (o != "" ? o : p) }')"
    w="${size%x*}"; h="${size#*x}"
    x=$((w / 2)); y_top=$((h * 30 / 100)); y_bottom=$((h * 75 / 100))
    sh_ dumpsys gfxinfo "$pkg" reset >/dev/null
    for ((i = 1; i <= SWIPES; i++)); do
        if ((i % 2)); then
            sh_ input swipe "$x" "$y_bottom" "$x" "$y_top" 120   # fling down the timeline
        else
            sh_ input swipe "$x" "$y_top" "$x" "$y_bottom" 120   # and back up
        fi
        # framestats keeps only the most recent ~120 frames (1 s at 120 Hz), so dump
        # after every swipe and merge by IntendedVsync.
        sleep 0.7
        sh_ dumpsys gfxinfo "$pkg" framestats >"$RAW/gfxinfo-$i.txt"
        dumps+=("$RAW/gfxinfo-$i.txt")
    done
    sleep 1
    sh_ dumpsys gfxinfo "$pkg" framestats >"$RAW/gfxinfo-final.txt"
    dumps+=("$RAW/gfxinfo-final.txt")
    python3 "$REPORT_PY" gfxinfo "${dumps[@]}" >"$RAW/gfxinfo.json"
    meta set gfxinfoPackage "$pkg"
    SUMMARY_ARGS+=(--gfxinfo "$RAW/gfxinfo.json")
}

if $RUN_GFXINFO; then
    if [[ " ${TARGET_KINDS[*]} " == *" native "* ]]; then
        run_gfxinfo "$NATIVE_PKG"
    else
        warn "native app not selected/installed; skipping the gfxinfo pass"
    fi
fi

# --------------------------------------------------------------------- report
SUMMARY_ARGS+=(--meta "$META" --out "$REPORT")
$ENFORCE && SUMMARY_ARGS+=(--enforce)
log "Writing $REPORT"
status=0
python3 "$REPORT_PY" summary "${SUMMARY_ARGS[@]}" || status=$?
if [[ $status -ne 0 ]]; then
    warn "one or more pass criteria failed (see the Pass criteria table in $REPORT)"
fi
exit "$status"
