package app.photovault.android.ui.memories

import app.photovault.android.MainDispatcherRule
import app.photovault.android.data.CoreError
import app.photovault.android.data.FileKind
import app.photovault.android.data.fake.FakeCore
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.advanceUntilIdle
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.rules.TemporaryFolder

@OptIn(ExperimentalCoroutinesApi::class)
class MemoryFeedTest {
    @get:Rule val main = MainDispatcherRule()
    @get:Rule val tmp = TemporaryFolder()

    private val now = 1_793_664_000_000_000L
    private val ml = MemoriesTestMl()
    private val feed = MemoryFeed({ ml }, io = main.dispatcher, nowUs = { now }, utcOffsetSecs = { 3600 })

    @Test
    fun loadsWithTheLocalOffsetAndPutsUnseenFirst() = runTest(main.dispatcher) {
        ml.memoryList = listOf(memory("a", seen = true), memory("b"), memory("c", seen = true), memory("d"))
        feed.refresh()
        assertEquals(listOf(now to 3600), ml.memoriesCalls)
        assertEquals(listOf("b", "d", "a", "c"), feed.memories.value.map { it.id })
    }

    @Test
    fun markSeenDimsAndMovesBackOnceAndTellsCore() = runTest(main.dispatcher) {
        ml.memoryList = listOf(memory("a"), memory("b"))
        feed.refresh()
        feed.markSeen("a")
        assertEquals(listOf("b", "a"), feed.memories.value.map { it.id })
        assertTrue(feed.memories.value.last().seen)
        feed.markSeen("a") // already seen: no second write
        assertEquals(listOf("a"), ml.seen)
    }

    @Test
    fun errorsKeepTheListAndClearForgetsIt() = runTest(main.dispatcher) {
        ml.memoryList = listOf(memory("a"))
        feed.refresh()
        ml.failMemories = CoreError.Storage("locked")
        feed.refresh()
        assertEquals(listOf("a"), feed.memories.value.map { it.id })
        feed.clear()
        assertTrue(feed.memories.value.isEmpty())
    }

    @Test
    fun findFallsBackToCore() = runTest(main.dispatcher) {
        ml.memoryList = listOf(memory("x"))
        assertEquals("x", feed.find("x")?.id)
        assertNull(feed.find("missing"))
        feed.clear()
        assertEquals("x", feed.findBlocking("x")?.id)
    }

    @Test
    fun rowViewModelStartsEmptyAndRefreshes() = runTest(main.dispatcher) {
        ml.memoryList = listOf(memory("old"))
        feed.refresh()
        ml.memoryList = listOf(memory("new"))
        val vm = MemoriesViewModel(feed)
        advanceUntilIdle()
        assertEquals(listOf("new"), vm.memories.value.map { it.id })
    }

    @Test
    fun storyMarksSeenAndPages() = runTest(main.dispatcher) {
        ml.memoryList = listOf(memory("a"), memory("b"))
        val vm = MemoryStoryViewModel(feed, FakeCore(latencyMs = 0, ioDispatcher = main.dispatcher), tmp.newFolder("m"), "a", startIndex = 1, io = main.dispatcher)
        advanceUntilIdle()
        val state = vm.state.value
        assertEquals("a", state.memory?.id)
        assertEquals(1, state.index)
        assertEquals(listOf("a"), ml.seen)
        assertTrue(vm.next())
        assertFalse(vm.next()) // last photo: the screen closes
        assertEquals(2, vm.state.value.index)
        vm.previous()
        vm.previous()
        vm.previous()
        assertEquals(0, vm.state.value.index)
        vm.jumpTo(99)
        assertEquals(2, vm.state.value.index)
    }

    @Test
    fun storyOfAMissingMemoryCloses() = runTest(main.dispatcher) {
        val vm = MemoryStoryViewModel(feed, FakeCore(latencyMs = 0, ioDispatcher = main.dispatcher), tmp.newFolder("m"), "gone", io = main.dispatcher)
        advanceUntilIdle()
        assertTrue(vm.state.value.missing)
        assertFalse(vm.next())
    }

    @Test
    fun storyOriginalsAreTrimmedAndVideosUseTheirThumbnail() = runTest(main.dispatcher) {
        val core = FakeCore(itemCount = 50, latencyMs = 0, ioDispatcher = main.dispatcher)
        core.addServer("photos.example.com")
        core.login("me@example.com", "secret")
        val photos = core.timelinePage(0, 50)
        val images = photos.filter { it.kind == FileKind.Image }.take(MemoryStoryViewModel.KEEP_FILES + 2)
        ml.memoryList = listOf(memory("a", items = images))
        val dir = tmp.newFolder("story")
        val vm = MemoryStoryViewModel(feed, core, dir, "a", io = main.dispatcher)
        advanceUntilIdle()
        images.forEach { assertTrue(vm.original(it)!!.length() > 0) }
        assertEquals(MemoryStoryViewModel.KEEP_FILES, dir.listFiles { f -> f.name.endsWith(".bin") }!!.size)
        assertNull(vm.original(item(9, FileKind.Video)))
    }

    @Test
    fun viewerSourcePagesTheMemory() {
        val items = (1L..5L).map { item(it) }
        var loads = 0
        val source = MemoryViewerSource(FakeCore(latencyMs = 0)) { loads++; items }
        assertEquals(5L, source.timelineCount())
        assertEquals(listOf(3L, 4L), source.timelinePage(2, 2).map { it.id })
        assertEquals(listOf(5L), source.timelinePage(4, 10).map { it.id })
        assertTrue(source.timelinePage(5, 10).isEmpty())
        assertTrue(source.timelinePage(-1, 10).isEmpty())
        assertEquals(1, loads)
    }

    @Test
    fun unseenFirstKeepsOrder() {
        val list = listOf(memory("1", seen = true), memory("2"), memory("3"))
        assertEquals(listOf("2", "3", "1"), MemoryFeed.unseenFirst(list).map { it.id })
        val allSeen = list.map { it.copy(seen = true) }
        assertEquals(allSeen, MemoryFeed.unseenFirst(allSeen))
    }
}
