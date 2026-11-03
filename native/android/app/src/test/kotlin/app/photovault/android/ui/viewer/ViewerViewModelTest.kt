package app.photovault.android.ui.viewer

import app.photovault.android.MainDispatcherRule
import app.photovault.android.data.CoreError
import app.photovault.android.data.FileKind
import app.photovault.android.data.TimelineStore
import app.photovault.android.data.fake.FakeCore
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.advanceUntilIdle
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Rule
import org.junit.Test
import org.junit.rules.TemporaryFolder

@OptIn(ExperimentalCoroutinesApi::class)
class ViewerViewModelTest {
    @get:Rule val main = MainDispatcherRule()
    @get:Rule val tmp = TemporaryFolder()

    private val core = FakeCore(itemCount = 300, latencyMs = 10, ioDispatcher = main.dispatcher)
    private val store = TimelineStore(core, io = main.dispatcher)

    private suspend fun vm(): ViewerViewModel {
        core.addServer("photos.example.com")
        core.login("me@example.com", "secret")
        store.refresh()
        return ViewerViewModel(core, store, tmp.newFolder("viewer"), io = main.dispatcher)
    }

    @Test
    fun originalsAreDownloadedOnceAndTrimmed() = runTest(main.dispatcher) {
        val vm = vm()
        val photos = vm.items.value.filter { it.kind == FileKind.Image }.take(ViewerViewModel.KEEP_FILES + 3)
        val first = vm.original(photos[0])
        assertTrue(first.length() > 0)
        assertEquals(first, vm.original(photos[0]))
        photos.drop(1).forEach { vm.original(it) }
        val kept = first.parentFile!!.listFiles { f -> f.name.endsWith(".bin") }!!
        assertEquals(ViewerViewModel.KEEP_FILES, kept.size)
        assertTrue(first.parentFile!!.listFiles { f -> f.name.endsWith(".part") }!!.isEmpty())
    }

    @Test
    fun demoVideosReportAClearError() = runTest(main.dispatcher) {
        val vm = vm()
        val video = vm.items.value.first { it.kind == FileKind.Video }
        try {
            vm.video(video)
            fail("expected an error")
        } catch (e: CoreError) {
            assertTrue(e.message!!.isNotBlank())
        }
    }

    @Test
    fun pagingFollowsThePager() = runTest(main.dispatcher) {
        val vm = vm()
        assertEquals(TimelineStore.CHUNK, vm.items.value.size)
        vm.onPage(150)
        advanceUntilIdle()
        assertEquals(300, vm.items.value.size)
    }
}
