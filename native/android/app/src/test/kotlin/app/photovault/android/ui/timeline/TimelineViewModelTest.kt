package app.photovault.android.ui.timeline

import app.photovault.android.MainDispatcherRule
import app.photovault.android.data.TimelineStore
import app.photovault.android.data.fake.FakeCore
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.test.advanceUntilIdle
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertSame
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import java.time.LocalDate
import java.time.ZoneOffset
import java.util.Locale

@OptIn(ExperimentalCoroutinesApi::class)
class TimelineViewModelTest {
    @get:Rule val main = MainDispatcherRule()

    private val now = 1_793_664_000_000_000L // 2026-11-03T00:00:00Z
    private val core = FakeCore(nowUs = now, itemCount = 640, latencyMs = 50, ioDispatcher = main.dispatcher)
    private val store = TimelineStore(core, io = main.dispatcher)
    private val labels = TimelineLabels("Today", "Yesterday", ZoneOffset.UTC, Locale.US) { LocalDate.of(2026, 11, 3) }

    private suspend fun vm(autoSync: Boolean = false): TimelineViewModel {
        core.addServer("photos.example.com")
        core.login("me@example.com", "secret")
        return TimelineViewModel(core, store, labels, io = main.dispatcher, compute = main.dispatcher, autoSync = autoSync)
    }

    @Test
    fun loadsTheFirstChunkWithHeaders() = runTest(main.dispatcher) {
        val vm = vm()
        advanceUntilIdle()
        val grid = vm.grid.value
        val cells = grid.entries.filterIsInstance<GridEntry.Cell>()
        assertEquals(TimelineStore.CHUNK, cells.size)
        assertTrue(grid.entries.first() is GridEntry.Header)
        assertEquals(ZoomLevels[DEFAULT_ZOOM_INDEX].columns, grid.columns)
        val status = vm.status.value
        assertTrue(status.loaded)
        assertFalse(status.empty)
        assertTrue(status.demo)
        assertEquals("me@example.com", status.account?.email)
        assertEquals("Demo vault", status.serverName)
    }

    @Test
    fun pagesAheadOfTheScroll() = runTest(main.dispatcher) {
        val vm = vm()
        advanceUntilIdle()
        vm.onVisible(150)
        advanceUntilIdle()
        assertEquals(2 * TimelineStore.CHUNK, store.items.value.size)
        vm.onVisible(10)
        advanceUntilIdle()
        assertEquals(2 * TimelineStore.CHUNK, store.items.value.size)
        vm.onVisible(630)
        advanceUntilIdle()
        assertEquals(640, store.items.value.size)
        assertEquals(640, vm.grid.value.entries.count { it is GridEntry.Cell })
    }

    @Test
    fun zoomKeepsEntriesWithinAGroupingAndRegroupsAcross() = runTest(main.dispatcher) {
        val vm = vm()
        advanceUntilIdle()
        val dayEntries = vm.grid.value.entries
        assertTrue(vm.zoomBy(-1))
        advanceUntilIdle()
        assertEquals(3, vm.grid.value.columns)
        assertSame(dayEntries, vm.grid.value.entries) // same grouping: no rebuild
        vm.zoomBy(+2)
        advanceUntilIdle()
        assertEquals(6, vm.grid.value.columns)
        assertTrue(vm.grid.value.entries.filterIsInstance<GridEntry.Header>().all { it.key.startsWith("m") })
        vm.zoomBy(+1)
        advanceUntilIdle()
        assertEquals(10, vm.grid.value.columns)
        assertTrue(vm.grid.value.entries.filterIsInstance<GridEntry.Header>().all { it.key.startsWith("y") })
        assertFalse(vm.zoomBy(+1)) // already the densest
        assertEquals(10, vm.grid.value.columns)
    }

    @Test
    fun syncReportsProgressAndFinishes() = runTest(main.dispatcher) {
        val vm = vm(autoSync = true)
        advanceUntilIdle()
        val status = vm.status.value
        assertFalse(status.syncing)
        assertTrue(status.progress?.done == true)
        assertEquals(640L, status.progress?.files)
    }

    @Test
    fun signOutClearsAndNavigates() = runTest(main.dispatcher) {
        val vm = vm()
        advanceUntilIdle()
        vm.signOut()
        advanceUntilIdle()
        assertEquals(TimelineEvent.SignedOut, vm.eventFlow.first())
        assertTrue(store.items.value.isEmpty())
        assertEquals(null, core.currentAccount())
    }
}
