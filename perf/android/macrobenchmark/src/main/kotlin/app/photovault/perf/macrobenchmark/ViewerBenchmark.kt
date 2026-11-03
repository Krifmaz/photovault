package app.photovault.perf.macrobenchmark

import androidx.benchmark.macro.StartupMode
import androidx.benchmark.macro.junit4.MacrobenchmarkRule
import androidx.test.ext.junit.runners.AndroidJUnit4
import app.photovault.perf.journeys.TargetApp
import app.photovault.perf.journeys.closeViewer
import app.photovault.perf.journeys.launchTarget
import app.photovault.perf.journeys.openViewer
import app.photovault.perf.journeys.swipeViewer
import app.photovault.perf.journeys.waitForTimeline
import org.junit.Assume.assumeTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

/**
 * Open the first photo (shared-element / hero transition), swipe five photos
 * forward and five back, then return to the timeline. The open and close
 * animations are part of the measured block on purpose.
 */
@RunWith(AndroidJUnit4::class)
class ViewerBenchmark {
    @get:Rule
    val rule = MacrobenchmarkRule()

    @Test
    fun openViewerAndSwipe() {
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
            openViewer()
            swipeViewer()
            closeViewer()
        }
    }
}
