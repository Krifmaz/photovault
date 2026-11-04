package app.photovault.perf.journeys

import android.graphics.Rect
import androidx.benchmark.macro.MacrobenchmarkScope
import androidx.test.uiautomator.By
import androidx.test.uiautomator.BySelector
import androidx.test.uiautomator.Direction
import androidx.test.uiautomator.UiDevice
import androidx.test.uiautomator.UiObject2
import androidx.test.uiautomator.Until
import java.util.regex.Pattern

/**
 * UiAutomator selectors. These are the contract with the two app tracks.
 *
 * Native (Compose): `Modifier.testTag(...)` on the nodes below, and
 * `Modifier.semantics { testTagsAsResourceId = true }` on a root composable so the
 * tags are exposed to UiAutomator as resource ids.
 *
 * Flutter: `Semantics(identifier: ...)`, which Flutter exposes on Android as the
 * view resource id. The home gallery already carries the identifier
 * `ente.photos.gallery.home_gallery.scroll` (inherited code); a `photovault.` spelling
 * is accepted too in case the fork renames it. Tiles and the viewer have no
 * identifier upstream, so the journeys fall back to coordinates inside the grid and
 * a screen-centre swipe; adding the optional identifiers below makes them exact.
 */
object Selectors {
    const val TIMELINE_GRID = "timeline_grid"
    const val PHOTO_TILE = "photo_tile"
    const val VIEWER_PAGER = "viewer_pager"

    val FLUTTER_GALLERY: Pattern =
        Pattern.compile("(photovault|ente\\.photos)\\.gallery\\.home_gallery\\.scroll")
    val FLUTTER_TILE: Pattern = Pattern.compile("photovault\\.gallery\\.tile(\\..*)?")
    val FLUTTER_VIEWER: Pattern = Pattern.compile("photovault\\.viewer\\.pager")

    fun timeline(kind: AppKind): BySelector = when (kind) {
        AppKind.NATIVE -> By.res(TIMELINE_GRID)
        AppKind.FLUTTER -> By.res(FLUTTER_GALLERY)
    }

    fun tile(kind: AppKind): BySelector = when (kind) {
        AppKind.NATIVE -> By.res(PHOTO_TILE)
        AppKind.FLUTTER -> By.res(FLUTTER_TILE)
    }

    fun viewer(kind: AppKind): BySelector = when (kind) {
        AppKind.NATIVE -> By.res(VIEWER_PAGER)
        AppKind.FLUTTER -> By.res(FLUTTER_VIEWER)
    }
}

private const val FIND_TIMEOUT_MS = 15_000L
private const val SHORT_TIMEOUT_MS = 3_000L

/** Launches the target app's launcher activity and waits for its first frame. */
fun MacrobenchmarkScope.launchTarget() {
    if (TargetApp.kind == AppKind.FLUTTER && TargetApp.flutterTrace) {
        startActivityAndWait { intent -> intent.putExtra("trace-systrace", true) }
    } else {
        startActivityAndWait()
    }
}

/**
 * Waits for the timeline grid. The app must already be signed in (or in local-only
 * mode) with enough photos to scroll; see perf/README.md "Device preparation".
 */
fun MacrobenchmarkScope.waitForTimeline(): UiObject2 {
    val kind = TargetApp.kind
    device.wait(Until.findObject(Selectors.timeline(kind)), FIND_TIMEOUT_MS)?.let { return it }
    if (kind == AppKind.FLUTTER) {
        // Fallback: the largest scrollable on screen is the gallery.
        device.wait(Until.findObjects(By.scrollable(true)), SHORT_TIMEOUT_MS)
            ?.maxByOrNull { it.visibleBounds.width() * it.visibleBounds.height() }
            ?.let { return it }
    }
    error(
        "Timeline grid not found in ${TargetApp.packageName} " +
            "(selector ${Selectors.timeline(kind)}). Is the app signed in with photos, " +
            "and does it expose the test tags described in perf/README.md?",
    )
}

/** Flings the timeline down and back up. This is the measured scroll journey. */
fun MacrobenchmarkScope.flingTimeline(times: Int = 3) {
    val grid = waitForTimeline()
    // Keep gestures away from the system gesture areas at the edges.
    grid.setGestureMargin(device.displayWidth / 5)
    repeat(times) {
        grid.fling(Direction.DOWN)
        device.waitForIdle()
    }
    repeat(times) {
        grid.fling(Direction.UP)
        device.waitForIdle()
    }
}

/** Opens the first visible photo in the viewer. */
fun MacrobenchmarkScope.openViewer() {
    val kind = TargetApp.kind
    val grid = waitForTimeline()
    val tile = device.findObject(Selectors.tile(kind))
    if (tile != null) {
        tile.click()
    } else {
        check(kind == AppKind.FLUTTER) {
            "No '${Selectors.PHOTO_TILE}' test tag found in ${TargetApp.packageName}."
        }
        // Flutter fallback: tap a point in the first column, below the app bar and
        // first section header.
        val b: Rect = grid.visibleBounds
        device.click(b.left + b.width() / 8, b.top + b.height() * 2 / 5)
    }
    val viewer = device.wait(Until.findObject(Selectors.viewer(kind)), SHORT_TIMEOUT_MS)
    if (viewer == null) {
        check(kind == AppKind.FLUTTER) {
            "Viewer ('${Selectors.VIEWER_PAGER}') did not open in ${TargetApp.packageName}."
        }
        // No identifier in the Flutter viewer: give the hero animation time to finish.
        Thread.sleep(1_000)
    }
    device.waitForIdle()
}

/** Swipes to the next photo [times] times, then back again. */
fun MacrobenchmarkScope.swipeViewer(times: Int = 5) {
    val pager = device.findObject(Selectors.viewer(TargetApp.kind))
    if (pager != null) {
        pager.setGestureMargin(device.displayWidth / 10)
        repeat(times) { pager.swipe(Direction.LEFT, 0.8f); device.waitForIdle() }
        repeat(times) { pager.swipe(Direction.RIGHT, 0.8f); device.waitForIdle() }
    } else {
        repeat(times) { device.horizontalSwipe(toLeft = true); device.waitForIdle() }
        repeat(times) { device.horizontalSwipe(toLeft = false); device.waitForIdle() }
    }
}

/** Leaves the viewer and waits for the timeline again. */
fun MacrobenchmarkScope.closeViewer() {
    device.pressBack()
    waitForTimeline()
}

private fun UiDevice.horizontalSwipe(toLeft: Boolean) {
    val y = displayHeight / 2
    val right = displayWidth * 85 / 100
    val left = displayWidth * 15 / 100
    // 10 steps of ~5 ms each: a quick swipe that the pager treats as a fling.
    if (toLeft) swipe(right, y, left, y, 10) else swipe(left, y, right, y, 10)
}
