package app.photovault.android.ui.trash

import app.photovault.android.MainDispatcherRule
import app.photovault.android.data.FileKind
import app.photovault.android.data.TimelineItem
import app.photovault.android.data.albums.TrashEntry
import app.photovault.android.ui.albums.signedInAlbums
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.advanceUntilIdle
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test

@OptIn(ExperimentalCoroutinesApi::class)
class TrashViewModelTest {
    @get:Rule val main = MainDispatcherRule()

    private val now = 1_793_664_000_000_000L
    private val day = 86_400_000_000L

    @Test
    fun syncsAndListsTheTrash() = runTest(main.dispatcher) {
        val (_, albums) = signedInAlbums(main.dispatcher, now)
        val vm = TrashViewModel(albums, io = main.dispatcher, nowUs = { now })
        advanceUntilIdle()
        val s = vm.state.value
        assertTrue(s.loaded)
        assertFalse(s.syncing)
        assertEquals(9, s.entries.size)
        assertEquals(29, daysLeft(s.entries.first(), now))
    }

    @Test
    fun restoreIntoAnAlbum() = runTest(main.dispatcher) {
        val (_, albums) = signedInAlbums(main.dispatcher, now)
        val vm = TrashViewModel(albums, io = main.dispatcher, nowUs = { now })
        advanceUntilIdle()
        val targets = vm.restoreTargets()
        assertTrue(targets.all { it.owned })
        val family = targets.first { it.collection.name == "Family" }
        val picked = vm.state.value.entries.take(2).map { it.item.id }
        picked.forEach(vm::toggle)
        vm.restoreSelected(family.id)
        advanceUntilIdle()
        assertEquals(7, vm.state.value.entries.size)
        assertTrue(vm.state.value.selection.isEmpty())
        val inFamily = albums.albumPage(family.id, 0, 1000).map { it.id }
        assertTrue(inFamily.containsAll(picked))
    }

    @Test
    fun deleteForGoodAndEmpty() = runTest(main.dispatcher) {
        val (_, albums) = signedInAlbums(main.dispatcher, now)
        val vm = TrashViewModel(albums, io = main.dispatcher, nowUs = { now })
        advanceUntilIdle()
        vm.toggle(vm.state.value.entries.first().item.id)
        vm.deleteSelected()
        advanceUntilIdle()
        assertEquals(8, vm.state.value.entries.size)
        vm.selectAll()
        assertEquals(8, vm.state.value.selection.size)
        vm.clearSelection()
        vm.emptyTrash()
        advanceUntilIdle()
        assertTrue(vm.state.value.entries.isEmpty())
        assertEquals(0L, albums.trashCount())
    }

    @Test
    fun daysLeftRoundsUpAndNeverGoesNegative() {
        val item = TimelineItem(1, 1, FileKind.Image, 0, 0, 0, null, false, "a.jpg")
        assertEquals(30, daysLeft(TrashEntry(item, now + 30 * day), now))
        assertEquals(1, daysLeft(TrashEntry(item, now + 1), now))
        assertEquals(0, daysLeft(TrashEntry(item, now - day), now))
    }
}
