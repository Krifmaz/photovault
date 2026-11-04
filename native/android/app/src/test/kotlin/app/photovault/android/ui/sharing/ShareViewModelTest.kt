package app.photovault.android.ui.sharing

import app.photovault.android.MainDispatcherRule
import app.photovault.android.data.albums.ShareRole
import app.photovault.android.ui.albums.signedInAlbums
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.advanceUntilIdle
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test

@OptIn(ExperimentalCoroutinesApi::class)
class ShareViewModelTest {
    @get:Rule val main = MainDispatcherRule()

    private val now = 1_793_664_000_000_000L

    private suspend fun vm(album: String): ShareViewModel {
        val (_, albums) = signedInAlbums(main.dispatcher, now)
        val id = albums.albums().first { it.collection.name == album }.id
        return ShareViewModel(albums, id, io = main.dispatcher, nowUs = { now })
    }

    @Test
    fun shareChangeRoleAndUnshare() = runTest(main.dispatcher) {
        val vm = vm("Holidays")
        advanceUntilIdle()
        assertTrue(vm.state.value.details!!.sharees.isEmpty())
        vm.share(" Kim@Example.com ", ShareRole.Viewer)
        advanceUntilIdle()
        assertEquals("Kim@Example.com", vm.state.value.addedEmail)
        vm.addedEmailConsumed()
        val s = vm.state.value.details!!.sharees.single()
        assertEquals("kim@example.com" to ShareRole.Viewer, s.email to s.role)
        vm.changeRole(s.email, ShareRole.Collaborator)
        advanceUntilIdle()
        assertEquals(ShareRole.Collaborator, vm.state.value.details!!.sharees.single().role)
        vm.unshare(s.email)
        advanceUntilIdle()
        assertTrue(vm.state.value.details!!.sharees.isEmpty())
    }

    @Test
    fun someoneWithoutAnAccountIsAnError() = runTest(main.dispatcher) {
        val vm = vm("Holidays")
        advanceUntilIdle()
        vm.share("unknown@example.com", ShareRole.Viewer)
        advanceUntilIdle()
        assertNotNull(vm.state.value.error)
        assertNull(vm.state.value.addedEmail)
        assertFalse(vm.state.value.busy)
    }

    @Test
    fun publicLinkLifecycle() = runTest(main.dispatcher) {
        val vm = vm("Holidays")
        advanceUntilIdle()
        assertNull(vm.state.value.details!!.publicLink)
        vm.createLink()
        advanceUntilIdle()
        val link = vm.state.value.details!!.publicLink!!
        assertTrue(link.url.contains('#'))
        assertEquals(0L, link.validTillUs)

        vm.setExpiry(LinkExpiry.Week)
        advanceUntilIdle()
        assertEquals(now + LinkExpiry.Week.durationUs, vm.state.value.details!!.publicLink!!.validTillUs)
        vm.setDeviceLimit(5)
        advanceUntilIdle()
        vm.setDownloads(false)
        advanceUntilIdle()
        vm.setCollect(true)
        advanceUntilIdle()
        vm.setPassword("hunter2")
        advanceUntilIdle()
        val l = vm.state.value.details!!.publicLink!!
        assertEquals(5, l.deviceLimit)
        assertFalse(l.enableDownload)
        assertTrue(l.enableCollect)
        assertTrue(l.passwordEnabled)

        vm.removePassword()
        advanceUntilIdle()
        assertFalse(vm.state.value.details!!.publicLink!!.passwordEnabled)
        vm.setExpiry(LinkExpiry.Never)
        advanceUntilIdle()
        assertEquals(0L, vm.state.value.details!!.publicLink!!.validTillUs)
        vm.removeLink()
        advanceUntilIdle()
        assertNull(vm.state.value.details!!.publicLink)
    }
}
