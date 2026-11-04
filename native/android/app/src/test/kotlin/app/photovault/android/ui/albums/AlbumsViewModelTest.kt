package app.photovault.android.ui.albums

import app.photovault.android.MainDispatcherRule
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.launch
import kotlinx.coroutines.test.advanceUntilIdle
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test

@OptIn(ExperimentalCoroutinesApi::class)
class AlbumsViewModelTest {
    @get:Rule val main = MainDispatcherRule()

    @Test
    fun splitsOwnAndSharedAlbumsAndHidesHiddenOnes() = runTest(main.dispatcher) {
        val (core, albums) = signedInAlbums(main.dispatcher)
        val vm = AlbumsViewModel(albums, core, io = main.dispatcher)
        advanceUntilIdle()
        val s = vm.state.value
        assertTrue(s.loaded)
        assertTrue(s.demo)
        assertEquals(listOf("Hikes"), s.shared.map { it.collection.name })
        assertTrue(s.mine.any { it.collection.name == "Family" && it.sharees.size == 2 && it.publicLink != null })
        assertFalse((s.mine + s.shared).any { it.hidden || it.isDefaultHidden })
        assertEquals(9L, s.trashCount)
    }

    @Test
    fun createAlbumOpensIt() = runTest(main.dispatcher) {
        val (core, albums) = signedInAlbums(main.dispatcher)
        val vm = AlbumsViewModel(albums, core, io = main.dispatcher)
        advanceUntilIdle()
        val opened = mutableListOf<AlbumsEvent>()
        val job = launch { vm.eventFlow.collect { opened += it } }
        vm.createAlbum("  Road trip ")
        advanceUntilIdle()
        val created = vm.state.value.mine.first { it.collection.name == "Road trip" }
        assertEquals(listOf<AlbumsEvent>(AlbumsEvent.Open(created.id)), opened)
        assertEquals(0L, created.collection.count)
        assertFalse(vm.state.value.creating)
        job.cancel()
    }

    @Test
    fun blankNamesAreIgnoredAndSyncReloads() = runTest(main.dispatcher) {
        val (core, albums) = signedInAlbums(main.dispatcher)
        val vm = AlbumsViewModel(albums, core, io = main.dispatcher)
        advanceUntilIdle()
        val before = vm.state.value.mine.size
        vm.createAlbum("   ")
        advanceUntilIdle()
        assertEquals(before, vm.state.value.mine.size)
        vm.sync()
        advanceUntilIdle()
        assertFalse(vm.state.value.syncing)
        assertNull(vm.state.value.error)
        assertEquals(before, vm.state.first().mine.size)
    }
}
