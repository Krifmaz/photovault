package app.photovault.perf.macrobenchmark

import androidx.benchmark.macro.StartupMode
import androidx.benchmark.macro.junit4.MacrobenchmarkRule
import androidx.test.ext.junit.runners.AndroidJUnit4
import app.photovault.perf.journeys.TargetApp
import app.photovault.perf.journeys.flingTimeline
import app.photovault.perf.journeys.launchTarget
import app.photovault.perf.journeys.waitForTimeline
import org.junit.Assume.assumeTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

/**
 * Fling the timeline grid down three times and back up three times.
 *
 * Pass (native, 120 Hz, release): frameDurationCpuMs P99 <= 8.33 and
 * frameOverrunMs P99 <= 0. See perf/README.md.
 */
@RunWith(AndroidJUnit4::class)
class TimelineScrollBenchmark {
    @get:Rule
    val rule = MacrobenchmarkRule()

    @Test
    fun timelineFling() {
        assumeTrue("requireTimeline=false: smoke run, no library to scroll", TargetApp.requireTimeline)
        rule.measureRepeated(
            packageName = TargetApp.packageName,
            metrics = frameMetrics(),
            compilationMode = TargetApp.compilationMode,
            startupMode = StartupMode.WARM,
            iterations = TargetApp.iterations,
            setupBlock = {
                pressHome()
                launchTarget()
                waitForTimeline()
            },
        ) {
            flingTimeline()
        }
    }
}
