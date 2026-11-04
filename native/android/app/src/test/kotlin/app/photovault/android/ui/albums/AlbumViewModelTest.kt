package app.photovault.android.ui.albums

import app.photovault.android.MainDispatcherRule
import app.photovault.android.data.TimelineStore
import app.photovault.android.data.albums.FakeAlbums
import app.photovault.android.data.fake.FakeCore
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.launch
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.advanceUntilIdle
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test

@OptIn(ExperimentalCoroutinesApi::class)
class AlbumViewModelTest {
    @get:Rule val main = MainDispatcherRule()

    private suspend fun TestScope.open(name: String): Triple<AlbumViewModel, FakeAlbums, FakeCore> {
        val (core, albums) = signedInAlbums(main.dispatcher)
        val id = (albums.albums() + albums.hiddenAlbums()).first { it.collection.name == name }.id
        val store = TimelineStore(
            core,
            io = main.dispatcher,
            countOf = { albums.albumCount(id) },
            pageOf = { o, l -> albums.albumPage(id, o, l) },
        )
        val vm = AlbumViewModel(albums, store, id, io = main.dispatcher)
        advanceUntilIdle()
        return Triple(vm, albums, core)
    }

    @Test
    fun loadsTheAlbumsPhotosNewestFirst() = runTest(main.dispatcher) {
        val (vm, albums) = open("Holidays")
        val s = vm.state.value
        assertTrue(s.loaded)
        assertEquals("Holidays", s.details?.collection?.name)
        val items = vm.items.value
        assertEquals(albums.albumCount(vm.collectionId).toInt(), items.size)
        assertTrue(items.all { it.collectionId == vm.collectionId })
        assertEquals(items.sortedByDescending { it.takenAtUs }, items)
    }

    @Test
    fun selectedPhotosGoToTheTrash() = runTest(main.dispatcher) {
        val (vm, albums) = open("Holidays")
        val events = mutableListOf<AlbumEvent>()
        val job = launch { vm.eventFlow.collect { events += it } }
        val before = vm.items.value.size
        val picked = vm.items.value.take(3).map { it.id }
        picked.forEach(vm::toggle)
        assertEquals(picked.toSet(), vm.state.value.selection)
        vm.trashSelected()
        advanceUntilIdle()
        assertEquals(before - 3, vm.items.value.size)
        assertTrue(vm.state.value.selection.isEmpty())
        assertEquals(listOf<AlbumEvent>(AlbumEvent.Done(DoneKind.Trashed, 3)), events)
        assertTrue(albums.trashPage(0, 100).map { it.item.id }.containsAll(picked))
        job.cancel()
    }

    @Test
    fun addToAnotherAlbumAndToANewOne() = runTest(main.dispatcher) {
        val (vm, albums) = open("Camera")
        val targets = vm.targets()
        assertFalse(targets.any { it.id == vm.collectionId || it.isDefaultHidden })
        val family = targets.first { it.collection.name == "Family" }
        val pick = vm.items.value.first().id
        vm.toggle(pick)
        vm.addSelectedTo(family.id)
        advanceUntilIdle()
        assertTrue(albums.albumPage(family.id, 0, 1000).any { it.id == pick })
        vm.toggle(pick)
        vm.addSelectedToNew("Best of")
        advanceUntilIdle()
        val best = albums.albums().first { it.collection.name == "Best of" }
        assertEquals(1L, best.collection.count)
    }

    @Test
    fun renameHideAndDelete() = runTest(main.dispatcher) {
        val (vm, albums) = open("Screenshots")
        val events = mutableListOf<AlbumEvent>()
        val job = launch { vm.eventFlow.collect { events += it } }
        vm.rename("Screens")
        advanceUntilIdle()
        assertEquals("Screens", vm.state.value.details?.collection?.name)
        vm.setHidden(true)
        advanceUntilIdle()
        assertTrue(vm.state.value.details!!.hidden)
        assertTrue(albums.hiddenAlbums().any { it.id == vm.collectionId })
        val trashBefore = albums.trashCount()
        vm.delete(keepFiles = false)
        advanceUntilIdle()
        assertEquals(listOf<AlbumEvent>(AlbumEvent.Gone), events)
        assertTrue(albums.albumDetails(vm.collectionId) == null)
        assertTrue(albums.trashCount() > trashBefore)
        job.cancel()
    }

    @Test
    fun sharedWithMeCanBeLeftButNotRenamed() = runTest(main.dispatcher) {
        val (vm, albums) = open("Hikes")
        assertFalse(vm.state.value.details!!.owned)
        vm.rename("Mine now")
        advanceUntilIdle()
        assertNotNull(vm.state.value.error)
        vm.clearError()
        vm.leave()
        advanceUntilIdle()
        assertTrue(albums.albums().none { it.id == vm.collectionId })
    }

    @Test
    fun hiddenPhotosCanBeUnhidden() = runTest(main.dispatcher) {
        val (vm, albums) = open("Hidden")
        assertTrue(vm.state.value.details!!.isDefaultHidden)
        val pick = vm.items.value.first().id
        val camera = albums.albums().first { it.collection.name == "Camera" }
        vm.toggle(pick)
        vm.unhideSelected(camera.id)
        advanceUntilIdle()
        assertTrue(vm.items.value.none { it.id == pick })
        assertTrue(albums.albumPage(camera.id, 0, 1000).any { it.id == pick })
    }
}
