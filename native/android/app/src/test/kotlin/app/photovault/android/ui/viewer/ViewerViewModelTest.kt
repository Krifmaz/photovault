package app.photovault.android.ui.viewer

import app.photovault.android.MainDispatcherRule
import app.photovault.android.data.CoreError
import app.photovault.android.data.FileKind
import app.photovault.android.data.TimelineStore
import app.photovault.android.data.VideoStream
import app.photovault.android.data.fake.FakeCore
import app.photovault.android.data.fake.FakeVideoRepository
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.advanceUntilIdle
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
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

    @Test
    fun aRestoredViewerLoadsTheTimelineUpToItsPhoto() = runTest(main.dispatcher) {
        // Process death with the viewer on top: nothing else fills the shared store.
        core.addServer("photos.example.com")
        core.login("me@example.com", "secret")
        val vm = ViewerViewModel(core, store, tmp.newFolder("viewer"), io = main.dispatcher, startIndex = 250)
        advanceUntilIdle()
        assertTrue(vm.loaded.value)
        assertTrue(vm.items.value.size > 250)
    }

    @Test
    fun videosPlayCoresStreamWhenThereIsOne() = runTest(main.dispatcher) {
        core.addServer("photos.example.com")
        core.login("me@example.com", "secret")
        store.refresh()
        val videos = store.items.value.filter { it.kind == FileKind.Video }.take(3)
        val url = "http://127.0.0.1:4242/token/stream/index.m3u8"
        val repo = FakeVideoRepository(
            streams = mapOf(videos[0].id to VideoStream(url, 1280, 720, 61_000, 9_000_000)),
            failing = setOf(videos[2].id),
        )
        val vm = ViewerViewModel(core, store, tmp.newFolder("viewer"), io = main.dispatcher, videos = repo)
        assertEquals(VideoSource(url, isHls = true), vm.video(videos[0]))
        // No stream (even without hasStream, core is asked: a shared video may have one): the original,
        // which the demo library cannot provide.
        try {
            vm.video(videos[1])
            fail("expected the demo's download error")
        } catch (e: CoreError) {
            assertTrue(e.message!!.isNotBlank())
        }
        // A failing stream falls back to the original as well.
        try {
            vm.video(videos[2])
            fail("expected the demo's download error")
        } catch (e: CoreError.Network) {
            assertFalse(e.message!!.contains("stream unavailable"))
        }
        assertEquals(videos.map { it.id }, repo.requested)
    }

    @Test
    fun closingThePlayerEndsCoresStreams() = runTest(main.dispatcher) {
        val repo = FakeVideoRepository()
        val vm = ViewerViewModel(core, store, tmp.newFolder("viewer"), io = main.dispatcher, videos = repo)
        vm.playback.release()
        assertEquals(1, repo.stopCount)
    }
}
