#!/usr/bin/env python3
"""Parsing and reporting helpers for perf/scripts/device-120hz.sh.

Subcommands (all read files written by the shell script; no device access here):

  refresh  --display FILE --sf FILE
      Parse `dumpsys display` and `dumpsys SurfaceFlinger`; print JSON with the
      supported refresh rates, the peak, and the current rate if it can be found.

  gfxinfo  --budget-ms 8.33 --pkg PKG FILE [FILE ...]
      Merge several `dumpsys gfxinfo PKG framestats` dumps taken during one scripted
      interaction (frames are de-duplicated by IntendedVsync, because each dump only
      holds the most recent ~120 frames). Print JSON with the system summary (total
      frames, janky %, percentiles) and per-frame p50/p90/p99 computed from framestats.

  meta     FILE set KEY VALUE | setjson KEY JSONFILE | append KEY VALUE | appendjson KEY JSONFILE
      Build the run's metadata file step by step from the shell (creates FILE if needed).
      VALUE "true"/"false" become booleans.

  summary  --meta META.json --out REPORT.md [--macro APP=DIR ...] [--gfxinfo FILE.json]
           [--budget-ms 8.33] [--enforce]
      Write the Markdown report. With --enforce, exit 1 when a pass criterion fails.

Standard library only. Python 3.8+.
"""

from __future__ import annotations

import argparse
import json
import math
import re
import sys
from pathlib import Path

# ---------------------------------------------------------------- pass criteria
# Kept in sync with perf/README.md ("Pass criteria").
FRAME_BUDGET_MS = 1000.0 / 120.0  # 8.33 ms
JANK_MAX_PCT = 1.0
STARTUP_TARGETS_MS = {
    # (app kind, benchmark name): median timeToInitialDisplayMs
    ("native", "startupCold"): 500.0,
    ("native", "startupWarm"): 200.0,
    ("flutter", "startupCold"): 800.0,
    ("flutter", "startupWarm"): 300.0,
}
FRAME_BENCHMARKS = ("timelineFling", "openViewerAndSwipe")


# ------------------------------------------------------------------- helpers
def percentile(values: list[float], pct: float) -> float | None:
    """Nearest-rank percentile (pct in 0..100)."""
    if not values:
        return None
    ordered = sorted(values)
    rank = max(1, math.ceil(pct / 100.0 * len(ordered)))
    return ordered[min(rank, len(ordered)) - 1]


def fmt(value, digits: int = 2) -> str:
    if value is None:
        return "n/a"
    if isinstance(value, float):
        return f"{value:.{digits}f}"
    return str(value)


def read(path: str | Path) -> str:
    return Path(path).read_text(encoding="utf-8", errors="replace")


# ------------------------------------------------------------------ refresh
def parse_refresh(display: str, sf: str) -> dict:
    rates: set[float] = set()
    for m in re.finditer(r"\bfps=([0-9]+(?:\.[0-9]+)?)", display):
        rates.add(round(float(m.group(1)), 2))
    for m in re.finditer(r"supportedRefreshRates \[([^\]]*)\]", display):
        for part in m.group(1).split(","):
            part = part.strip()
            if part:
                try:
                    rates.add(round(float(part), 2))
                except ValueError:
                    pass
    rates = {r for r in rates if r > 1.0}

    current = None
    source = None
    candidates = [
        (sf, r"refresh-rate\s*:\s*([0-9]+(?:\.[0-9]+)?)", "SurfaceFlinger refresh-rate"),
        (sf, r"renderRate=([0-9]+(?:\.[0-9]+)?)", "SurfaceFlinger renderRate"),
        (display, r"renderFrameRate[ =]([0-9]+(?:\.[0-9]+)?)", "display renderFrameRate"),
    ]
    for text, pattern, label in candidates:
        m = re.search(pattern, text)
        if m:
            current, source = round(float(m.group(1)), 2), label
            break
    if current is None:
        m = re.search(r"DisplayDeviceInfo\{.*?\bmodeId (\d+)", display)
        if m:
            mode_id = m.group(1)
            mm = re.search(r"\{id=" + mode_id + r", [^}]*?fps=([0-9]+(?:\.[0-9]+)?)", display)
            if mm:
                current, source = round(float(mm.group(1)), 2), f"display modeId {mode_id}"

    raw_lines = [
        line.strip()
        for line in sf.splitlines()
        if re.search(r"refresh-rate|renderRate|activeMode|vsync period", line, re.I)
    ][:6]
    return {
        "supportedHz": sorted(rates),
        "maxHz": max(rates) if rates else None,
        "currentHz": current,
        "currentSource": source,
        "rawSurfaceFlinger": raw_lines,
    }


# ------------------------------------------------------------------ gfxinfo
def parse_gfxinfo_summary(text: str) -> dict:
    out: dict = {}
    m = re.search(r"Total frames rendered:\s*(\d+)", text)
    if m:
        out["totalFrames"] = int(m.group(1))
    m = re.search(r"Janky frames:\s*(\d+)\s*\(([0-9.]+)%\)", text)
    if m:
        out["jankyFrames"] = int(m.group(1))
        out["jankyPct"] = float(m.group(2))
    m = re.search(r"Janky frames \(legacy\):\s*(\d+)\s*\(([0-9.]+)%\)", text)
    if m:
        out["jankyFramesLegacy"] = int(m.group(1))
        out["jankyPctLegacy"] = float(m.group(2))
    for p in ("50", "90", "95", "99"):
        m = re.search(rf"^\s*{p}th percentile:\s*(\d+)ms", text, re.M)
        if m:
            out[f"p{p}Ms"] = int(m.group(1))
    for key, label in (
        ("missedVsync", "Number Missed Vsync"),
        ("highInputLatency", "Number High input latency"),
        ("slowUiThread", "Number Slow UI thread"),
        ("slowBitmapUploads", "Number Slow bitmap uploads"),
        ("slowDrawCommands", "Number Slow issue draw commands"),
        ("frameDeadlineMissed", "Number Frame deadline missed"),
    ):
        m = re.search(re.escape(label) + r":\s*(\d+)", text)
        if m:
            out[key] = int(m.group(1))
    return out


def parse_framestats(text: str) -> dict[int, dict]:
    """Return {IntendedVsync: row} for every PROFILEDATA block in one dump."""
    frames: dict[int, dict] = {}
    for block in re.findall(r"---PROFILEDATA---\n(.*?)---PROFILEDATA---", text, re.S):
        lines = [l for l in block.strip().splitlines() if l.strip()]
        if not lines:
            continue
        header = [h.strip() for h in lines[0].rstrip(",").split(",")]
        if "IntendedVsync" not in header or "FrameCompleted" not in header:
            continue
        idx = {name: i for i, name in enumerate(header)}
        for line in lines[1:]:
            cols = [c.strip() for c in line.rstrip(",").split(",")]
            if len(cols) < len(header):
                continue
            try:
                row = {name: int(cols[i]) for name, i in idx.items()}
            except ValueError:
                continue
            frames[row["IntendedVsync"]] = row
    return frames


def analyse_gfxinfo(paths: list[str], budget_ms: float) -> dict:
    merged: dict[int, dict] = {}
    summary: dict = {}
    for p in paths:
        text = read(p)
        merged.update(parse_framestats(text))
        summary = parse_gfxinfo_summary(text) or summary  # the last dump covers everything
    durations = []
    flagged = 0
    for row in merged.values():
        if row.get("Flags", 0) != 0:
            flagged += 1  # layout/resize/etc. frames, excluded like HWUI does
            continue
        if row["FrameCompleted"] <= 0 or row["FrameCompleted"] < row["IntendedVsync"]:
            continue
        durations.append((row["FrameCompleted"] - row["IntendedVsync"]) / 1e6)
    over = sum(1 for d in durations if d > budget_ms)
    total = summary.get("totalFrames")
    return {
        "budgetMs": budget_ms,
        "summary": summary,
        "framestats": {
            "frames": len(durations),
            "flaggedExcluded": flagged,
            "coveragePct": (100.0 * (len(durations) + flagged) / total) if total else None,
            "p50Ms": percentile(durations, 50),
            "p90Ms": percentile(durations, 90),
            "p99Ms": percentile(durations, 99),
            "maxMs": max(durations) if durations else None,
            "overBudgetPct": (100.0 * over / len(durations)) if durations else None,
        },
    }


# ------------------------------------------------------------------ summary
def load_macro(directory: Path) -> list[dict]:
    rows = []
    for f in sorted(directory.rglob("*benchmarkData.json")):
        try:
            data = json.loads(read(f))
        except json.JSONDecodeError:
            continue
        for b in data.get("benchmarks", []):
            rows.append(b)
    return rows


def macro_metric_rows(benchmark: dict) -> list[tuple[str, dict]]:
    out = []
    for section in ("metrics", "sampledMetrics"):
        for name, values in (benchmark.get(section) or {}).items():
            out.append((name, values))
    return out


def evaluate_macro(kind: str, benchmarks: list[dict], budget_ms: float) -> list[tuple[str, bool, str]]:
    checks = []
    for b in benchmarks:
        name = b.get("name", "")
        metrics = dict(macro_metric_rows(b))
        target = STARTUP_TARGETS_MS.get((kind, name))
        if target is not None and "timeToInitialDisplayMs" in metrics:
            median = metrics["timeToInitialDisplayMs"].get("median")
            checks.append(
                (f"{kind} {name} median TTID <= {target:.0f} ms", median is not None and median <= target, f"{fmt(median, 1)} ms")
            )
        if kind == "native" and name in FRAME_BENCHMARKS:
            cpu = metrics.get("frameDurationCpuMs", {})
            p99 = cpu.get("P99")
            checks.append(
                (f"native {name} frameDurationCpuMs P99 <= {budget_ms:.2f} ms", p99 is not None and p99 <= budget_ms, f"{fmt(p99)} ms")
            )
            over = metrics.get("frameOverrunMs", {})
            if over:
                op99 = over.get("P99")
                checks.append((f"native {name} frameOverrunMs P99 <= 0", op99 is not None and op99 <= 0, f"{fmt(op99)} ms"))
    return checks


def write_summary(args) -> int:
    meta = json.loads(read(args.meta))
    budget = args.budget_ms
    lines: list[str] = []
    checks: list[tuple[str, bool, str]] = []

    lines += [
        f"# Perf run {meta.get('date', '')} on {meta.get('model', 'unknown device')}",
        "",
        "| | |",
        "| --- | --- |",
        f"| Device | {meta.get('manufacturer', '')} {meta.get('model', '')} (`{meta.get('serial', '')}`) |",
        f"| Android | {meta.get('release', '')} (API {meta.get('sdk', '')}) |",
        f"| Build | `{meta.get('fingerprint', '')}` |",
        f"| Mode | {'emulator (numbers are not 120 Hz numbers)' if meta.get('emulator') else 'physical device'} |",
        f"| Frame budget | {budget:.2f} ms |",
    ]
    for label, key in (("Refresh before", "refreshBefore"), ("Refresh after forcing", "refreshForced"), ("Refresh during gfxinfo run", "refreshDuring")):
        r = meta.get(key)
        if r:
            lines.append(
                f"| {label} | current {fmt(r.get('currentHz'))} Hz ({r.get('currentSource') or 'not found'}), "
                f"peak {fmt(r.get('maxHz'))} Hz, supported {r.get('supportedHz')} |"
            )
    for app in meta.get("apps", []):
        lines.append(f"| App `{app.get('package')}` | version {app.get('version', '?')}, debuggable={str(app.get('debuggable')).lower()} |")
    lines.append("")

    if not meta.get("emulator"):
        during = (meta.get("refreshDuring") or meta.get("refreshForced") or {}).get("currentHz")
        if during is not None:
            checks.append(("display ran at >= 119 Hz during the run", during >= 119.0, f"{fmt(during)} Hz"))

    for spec in args.macro or []:
        kind, _, directory = spec.partition("=")
        benchmarks = load_macro(Path(directory))
        lines += [f"## Macrobenchmark: {kind}", ""]
        if not benchmarks:
            lines += ["No benchmarkData.json found (run failed or was skipped; see the raw log).", ""]
            continue
        lines += ["| Benchmark | Metric | median / P50 | P90 | P99 | max |", "| --- | --- | --- | --- | --- | --- |"]
        for b in benchmarks:
            for name, v in macro_metric_rows(b):
                mid = v.get("median", v.get("P50"))
                lines.append(
                    f"| {b.get('name')} | {name} | {fmt(mid)} | {fmt(v.get('P90'))} | {fmt(v.get('P99'))} | {fmt(v.get('maximum'))} |"
                )
        lines.append("")
        if kind == "flutter":
            lines += [
                "Flutter frames are not visible to FrameTimingMetric; the build/raster "
                "percentiles for the Flutter app come from perf/flutter (integration_test, --profile).",
                "",
            ]
        checks += evaluate_macro(kind, benchmarks, budget)

    if args.gfxinfo:
        g = json.loads(read(args.gfxinfo))
        s, fs = g.get("summary", {}), g.get("framestats", {})
        lines += [
            f"## gfxinfo framestats: {meta.get('gfxinfoPackage', 'native app')} (scripted adb swipes)",
            "",
            "| | |",
            "| --- | --- |",
            f"| Frames rendered (system) | {s.get('totalFrames', 'n/a')} |",
            f"| Janky frames (system, deadline based) | {s.get('jankyFrames', 'n/a')} ({fmt(s.get('jankyPct'))} %) |",
            f"| Janky frames (legacy) | {s.get('jankyFramesLegacy', 'n/a')} ({fmt(s.get('jankyPctLegacy'))} %) |",
            f"| System percentiles p50 / p90 / p99 | {s.get('p50Ms', 'n/a')} / {s.get('p90Ms', 'n/a')} / {s.get('p99Ms', 'n/a')} ms |",
            f"| framestats frames analysed | {fs.get('frames')} (coverage {fmt(fs.get('coveragePct'), 0)} %, {fs.get('flaggedExcluded')} flagged excluded) |",
            f"| framestats p50 / p90 / p99 / max | {fmt(fs.get('p50Ms'))} / {fmt(fs.get('p90Ms'))} / {fmt(fs.get('p99Ms'))} / {fmt(fs.get('maxMs'))} ms |",
            f"| Frames over {budget:.2f} ms | {fmt(fs.get('overBudgetPct'))} % |",
            "",
            "Frame time = FrameCompleted - IntendedVsync from framestats (HWUI's own definition).",
            "",
        ]
        jank = s.get("jankyPct")
        checks.append((f"gfxinfo janky frames < {JANK_MAX_PCT:.0f} %", jank is not None and jank < JANK_MAX_PCT, f"{fmt(jank)} %"))
        p99 = fs.get("p99Ms")
        checks.append((f"gfxinfo framestats p99 <= {budget:.2f} ms", p99 is not None and p99 <= budget, f"{fmt(p99)} ms"))

    if meta.get("failures"):
        lines += ["## Failures", ""] + [f"- {f}" for f in meta["failures"]] + [""]
        checks.append(("all instrumentation runs completed", False, "; ".join(meta["failures"])))

    lines += ["## Pass criteria", "", "| Check | Result | Value |", "| --- | --- | --- |"]
    for name, ok, value in checks:
        lines.append(f"| {name} | {'PASS' if ok else 'FAIL'} | {value} |")
    if meta.get("emulator"):
        lines += ["", "Emulator run: informational only. The final check is a release build on a real 120 Hz phone."]
    lines.append("")

    Path(args.out).write_text("\n".join(lines), encoding="utf-8")
    print("\n".join(lines))
    failed = [c for c in checks if not c[1]]
    return 1 if (args.enforce and failed) else 0


# --------------------------------------------------------------------- meta
def update_meta(path: str, op: str, key: str, value: str) -> None:
    p = Path(path)
    data = json.loads(read(p)) if p.exists() and p.stat().st_size else {}
    if op in ("setjson", "appendjson"):
        parsed = json.loads(read(value))
    elif value in ("true", "false"):
        parsed = value == "true"
    else:
        parsed = value
    if op in ("set", "setjson"):
        data[key] = parsed
    elif op in ("append", "appendjson"):
        data.setdefault(key, []).append(parsed)
    else:
        raise SystemExit(f"unknown meta op {op}")
    p.write_text(json.dumps(data, indent=2), encoding="utf-8")


# --------------------------------------------------------------------- main
def main(argv: list[str]) -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)

    r = sub.add_parser("refresh")
    r.add_argument("--display", required=True)
    r.add_argument("--sf", required=True)

    g = sub.add_parser("gfxinfo")
    g.add_argument("--budget-ms", type=float, default=FRAME_BUDGET_MS)
    g.add_argument("files", nargs="+")

    m = sub.add_parser("meta")
    m.add_argument("file")
    m.add_argument("op", choices=["set", "setjson", "append", "appendjson"])
    m.add_argument("key")
    m.add_argument("value")

    s = sub.add_parser("summary")
    s.add_argument("--meta", required=True)
    s.add_argument("--out", required=True)
    s.add_argument("--macro", action="append", help="KIND=DIR, KIND is native or flutter")
    s.add_argument("--gfxinfo")
    s.add_argument("--budget-ms", type=float, default=FRAME_BUDGET_MS)
    s.add_argument("--enforce", action="store_true")

    args = ap.parse_args(argv)
    if args.cmd == "refresh":
        print(json.dumps(parse_refresh(read(args.display), read(args.sf))))
        return 0
    if args.cmd == "gfxinfo":
        print(json.dumps(analyse_gfxinfo(args.files, args.budget_ms)))
        return 0
    if args.cmd == "meta":
        update_meta(args.file, args.op, args.key, args.value)
        return 0
    return write_summary(args)


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
