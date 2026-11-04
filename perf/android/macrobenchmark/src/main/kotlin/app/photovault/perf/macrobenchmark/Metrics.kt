@file:OptIn(ExperimentalMetricApi::class)

package app.photovault.perf.macrobenchmark

import androidx.benchmark.macro.ExperimentalMetricApi
import androidx.benchmark.macro.FrameTimingMetric
import androidx.benchmark.macro.Metric
import androidx.benchmark.macro.TraceSectionMetric
import app.photovault.perf.journeys.AppKind
import app.photovault.perf.journeys.TargetApp

/**
 * Frame metrics for scroll and animation journeys.
 *
 * Native: [FrameTimingMetric] (frameDurationCpuMs, frameOverrunMs on API 31+). It
 * reads the HWUI RenderThread and SurfaceFlinger FrameTimeline for the app's
 * window, which is exactly how a Compose app draws.
 *
 * Flutter: Flutter draws into its own surface on its own UI and raster threads, so
 * [FrameTimingMetric] does not see Flutter frames. Instead:
 * - `Choreographer#doFrame%` count: how many vsync callbacks the app's embedding took
 *   during the journey. Over a fixed journey this should scale with the refresh rate
 *   (about twice as many at 120 Hz as at 60 Hz); a drop means the app fell back to a
 *   lower rate or stalled.
 * - With `flutterTrace=true` and a `--profile` build: the engine's per-frame raster
 *   (`GPURasterizer::Draw`) and UI (`Animator::BeginFrame`) sections, average and max.
 * The authoritative Flutter build/raster percentiles come from perf/flutter
 * (integration_test in --profile mode).
 */
fun frameMetrics(): List<Metric> = when (TargetApp.kind) {
    AppKind.NATIVE -> listOf(FrameTimingMetric())
    AppKind.FLUTTER -> buildList<Metric> {
        add(
            TraceSectionMetric(
                sectionName = "Choreographer#doFrame%",
                mode = TraceSectionMetric.Mode.Count,
                label = "vsyncCallbacks",
            ),
        )
        if (TargetApp.flutterTrace) {
            add(TraceSectionMetric(sectionName = "GPURasterizer::Draw", mode = TraceSectionMetric.Mode.Average, label = "flutterRasterAvg"))
            add(TraceSectionMetric(sectionName = "GPURasterizer::Draw", mode = TraceSectionMetric.Mode.Max, label = "flutterRasterMax"))
            add(TraceSectionMetric(sectionName = "Animator::BeginFrame", mode = TraceSectionMetric.Mode.Average, label = "flutterUiAvg"))
            add(TraceSectionMetric(sectionName = "Animator::BeginFrame", mode = TraceSectionMetric.Mode.Max, label = "flutterUiMax"))
        }
    }
}
