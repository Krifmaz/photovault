package app.photovault.perf.baselineprofile

import androidx.benchmark.macro.junit4.BaselineProfileRule
import androidx.test.ext.junit.runners.AndroidJUnit4
import app.photovault.perf.journeys.AppKind
import app.photovault.perf.journeys.TargetApp
import app.photovault.perf.journeys.closeViewer
import app.photovault.perf.journeys.flingTimeline
import app.photovault.perf.journeys.launchTarget
import app.photovault.perf.journeys.openViewer
import app.photovault.perf.journeys.swipeViewer
import app.photovault.perf.journeys.waitForTimeline
import org.junit.Assume.assumeTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

/**
 * Generates the Baseline Profile and the startup profile for Photovault Native.
 *
 * Writes, in the test APK's media directory on the device
 * (/sdcard/Android/media/app.photovault.perf.baselineprofile/):
 *   BaselineProfileGenerator_generate-baseline-prof.txt
 *   BaselineProfileGenerator_generate-startup-prof.txt
 * perf/scripts/collect-baseline-profile.sh pulls them into native/android/app/src/main/.
 *
 * Needs Android 13+ (API 33) on a non-rooted device, or a rooted device on API 28+.
 * Use a release (non-debuggable) build of the app.
 */
@RunWith(AndroidJUnit4::class)
class BaselineProfileGenerator {
    @get:Rule
    val rule = BaselineProfileRule()

    @Test
    fun generate() {
        assumeTrue(
            "Baseline Profiles are generated for the native app only " +
                "(targetPackage=${TargetApp.packageName})",
            TargetApp.kind == AppKind.NATIVE,
        )
        rule.collect(
            packageName = TargetApp.packageName,
            includeInStartupProfile = true,
        ) {
            // Startup up to the timeline; everything here also lands in the startup profile.
            pressHome()
            launchTarget()
            waitForTimeline()
            // Hot paths after startup.
            flingTimeline(times = 2)
            openViewer()
            swipeViewer(times = 3)
            closeViewer()
        }
    }
}
