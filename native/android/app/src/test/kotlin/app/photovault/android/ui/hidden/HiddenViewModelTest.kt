package app.photovault.android.ui.hidden

import app.photovault.android.MainDispatcherRule
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
class HiddenViewModelTest {
    @get:Rule val main = MainDispatcherRule()

    @Test
    fun nothingIsReadUntilUnlocked() = runTest(main.dispatcher) {
        val (_, albums) = signedInAlbums(main.dispatcher)
        val vm = HiddenViewModel(albums, io = main.dispatcher)
        vm.refresh()
        advanceUntilIdle()
        assertFalse(vm.state.value.unlocked)
        assertFalse(vm.state.value.loaded)
        assertTrue(vm.state.value.albums.isEmpty())

        vm.unlocked()
        advanceUntilIdle()
        val list = vm.state.value.albums
        assertTrue(vm.state.value.loaded)
        // the hidden-photos collection first, then hidden albums
        assertTrue(list.first().isDefaultHidden)
        assertEquals(listOf("Private"), list.drop(1).map { it.collection.name })
    }

    @Test
    fun unhideMovesAnAlbumBack() = runTest(main.dispatcher) {
        val (_, albums) = signedInAlbums(main.dispatcher)
        val vm = HiddenViewModel(albums, io = main.dispatcher)
        vm.unlocked()
        advanceUntilIdle()
        val privateAlbum = vm.state.value.albums.first { it.collection.name == "Private" }
        vm.unhide(privateAlbum.id)
        advanceUntilIdle()
        assertTrue(vm.state.value.albums.none { it.id == privateAlbum.id })
        assertTrue(albums.albums().any { it.id == privateAlbum.id })
    }

    @Test
    fun theHiddenPhotosCollectionCannotBeUnhidden() = runTest(main.dispatcher) {
        val (_, albums) = signedInAlbums(main.dispatcher)
        val vm = HiddenViewModel(albums, io = main.dispatcher)
        vm.unlocked()
        advanceUntilIdle()
        val hiddenPhotos = vm.state.value.albums.first { it.isDefaultHidden }
        vm.unhide(hiddenPhotos.id)
        advanceUntilIdle()
        assertTrue(vm.state.value.error != null)
        assertTrue(vm.state.value.albums.any { it.id == hiddenPhotos.id })
    }
}
