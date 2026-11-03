package app.photovault.android.data.fake

import app.photovault.android.data.CoreError
import app.photovault.android.data.LoginStep
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class FakeCoreTest {
    private val now = 1_793_664_000_000_000L // 2026-11-03T00:00:00Z in µs

    @Test
    fun placeholderIsAValidDeterministicPng() {
        val a = PlaceholderPng.render(10_001, 64, 48)
        val b = PlaceholderPng.render(10_001, 64, 48)
        assertArrayEquals(a, b)
        assertEquals(0x89.toByte(), a[0])
        assertEquals("PNG", String(a, 1, 3, Charsets.US_ASCII))
        // IHDR width/height
        assertEquals(64, (a[16].toInt() and 0xFF shl 24) or (a[17].toInt() and 0xFF shl 16) or (a[18].toInt() and 0xFF shl 8) or (a[19].toInt() and 0xFF))
        assertEquals(48, a[23].toInt() and 0xFF)
    }

    @Test
    fun timelineIsNewestFirstAndPaged() = runTest {
        val core = FakeCore(nowUs = now, itemCount = 450, latencyMs = 0)
        core.addServer("photos.example.com")
        assertEquals(0L, core.timelineCount()) // not signed in yet
        val step = core.login("me@example.com", "secret")
        assertTrue(step is LoginStep.Done)
        assertEquals(450L, core.timelineCount())
        val first = core.timelinePage(0, 200)
        val second = core.timelinePage(200, 200)
        val third = core.timelinePage(400, 200)
        assertEquals(listOf(200, 200, 50), listOf(first.size, second.size, third.size))
        val all = first + second + third
        assertEquals(all.sortedByDescending { it.takenAtUs }, all)
        assertEquals(all.size, all.map { it.id }.toSet().size)
        assertTrue(core.thumbnail(all[0].id).size > 100)
    }

    @Test(expected = CoreError.Unreachable::class)
    fun unreachableHost() = runTest {
        FakeCore(nowUs = now, latencyMs = 0).fetchServerInfo("unreachable.example:8095")
    }
}
