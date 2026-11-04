package app.photovault.android.widget

import app.photovault.android.data.LoginStep
import app.photovault.android.data.fake.FakeCore
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import java.time.Instant
import java.time.ZoneOffset

class WidgetPicksTest {
    private val now = 1_793_664_000_000_000L // 2026-11-03T00:00:00Z in µs

    private suspend fun signedIn(count: Int = 2000): FakeCore {
        val core = FakeCore(nowUs = now, itemCount = count, latencyMs = 0)
        core.addServer("photos.example.com")
        assertTrue(core.login("me@example.com", "secret") is LoginStep.Done)
        return core
    }

    @Test
    fun signedOutShowsTheSignInMessage() = runTest {
        val core = FakeCore(nowUs = now, latencyMs = 0)
        assertEquals(WidgetPick.SignedOut, WidgetPicks.memories(core, now / 1000, 0, 0))
        assertEquals(WidgetPick.SignedOut, WidgetPicks.album(core, 1, 0))
    }

    @Test
    fun memoriesPickThisDayInAnEarlierYear() = runTest {
        val core = signedIn()
        val old = core.timelinePage(1500, 1).single()
        // "today" is exactly two years after that photo
        val at = Instant.ofEpochMilli(old.takenAtUs / 1000).atOffset(ZoneOffset.UTC).plusYears(2)
        val pick = WidgetPicks.memories(core, at.toInstant().toEpochMilli(), 0, 0)
        assertTrue(pick is WidgetPick.Photo)
        pick as WidgetPick.Photo
        val taken = Instant.ofEpochMilli(pick.item.takenAtUs / 1000).atOffset(ZoneOffset.UTC)
        assertEquals(at.monthValue to at.dayOfMonth, taken.monthValue to taken.dayOfMonth)
        assertTrue(pick.caption is Caption.YearsAgo)
        assertEquals(at.year - taken.year, (pick.caption as Caption.YearsAgo).years)
    }

    @Test
    fun memoriesFallBackToRecentAndRotate() = runTest {
        val core = signedIn(count = 50)
        // no photo of this library is from an earlier year on 2026-11-03
        val a = WidgetPicks.memories(core, now / 1000, 0, slot = 0) as WidgetPick.Photo
        val b = WidgetPicks.memories(core, now / 1000, 0, slot = 1) as WidgetPick.Photo
        assertEquals(Caption.Recent, a.caption)
        assertTrue(a.item.id != b.item.id)
    }

    @Test
    fun albumWidgetShowsThatAlbumOnly() = runTest {
        val core = signedIn(count = 100)
        val album = core.collections().first()
        val pick = WidgetPicks.album(core, album.id, 3) as WidgetPick.Photo
        assertEquals(album.id, pick.item.collectionId)
        assertEquals(Caption.Album(album.name), pick.caption)
        assertEquals(WidgetPick.AlbumMissing, WidgetPicks.album(core, 999_999, 0))
    }

    @Test
    fun fakeCoreHasPlacesForTheMap() = runTest {
        val core = signedIn(count = 90)
        val points = core.mapPoints()
        assertEquals(60, points.size)
        assertTrue(points.all { it.latitude in -90.0..90.0 && it.longitude in -180.0..180.0 })
    }
}
