package app.photovault.perf.macrobenchmark

import androidx.benchmark.macro.StartupMode
import androidx.benchmark.macro.StartupTimingMetric
import androidx.benchmark.macro.junit4.MacrobenchmarkRule
import androidx.test.ext.junit.runners.AndroidJUnit4
import app.photovault.perf.journeys.TargetApp
import app.photovault.perf.journeys.launchTarget
import app.photovault.perf.journeys.waitForTimeline
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

/**
 * Cold and warm startup to the timeline, for either app.
 *
 * Reports timeToInitialDisplayMs (first frame) and, when the app calls
 * Activity.reportFullyDrawn() once the timeline shows photos, timeToFullDisplayMs.
 * Works the same for Flutter, because the first Flutter frame is the activity's
 * first frame.
 */
@RunWith(AndroidJUnit4::class)
class StartupBenchmark {
    @get:Rule
    val rule = MacrobenchmarkRule()

    @Test
    fun startupCold() = startup(StartupMode.COLD)

    @Test
    fun startupWarm() = startup(StartupMode.WARM)

    private fun startup(mode: StartupMode) = rule.measureRepeated(
        packageName = TargetApp.packageName,
        metrics = listOf(StartupTimingMetric()),
        compilationMode = TargetApp.compilationMode,
        startupMode = mode,
        iterations = TargetApp.iterations,
        setupBlock = { pressHome() },
    ) {
        launchTarget()
        if (TargetApp.requireTimeline) waitForTimeline()
    }
}
