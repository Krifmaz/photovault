package app.photovault.android.ui.viewer.video

import app.photovault.android.data.CoreError
import app.photovault.android.data.FileKind
import app.photovault.android.data.TimelineItem
import app.photovault.android.ui.viewer.VideoSource
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.advanceTimeBy
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

private class FakeEngine : PlaybackEngine {
    var listener: PlaybackEngine.Listener? = null
    val calls = mutableListOf<String>()
    var loaded: VideoSource? = null
    var muted = false
    var released = false
    override var positionMs = 0L
    override var durationMs = 0L
    override var bufferedMs = 0L

    override fun setListener(listener: PlaybackEngine.Listener?) {
        this.listener = listener
    }

    override fun load(source: VideoSource) {
        loaded = source
        calls += "load:${source.uri}"
    }

    override fun play() {
        calls += "play"
    }

    override fun pause() {
        calls += "pause"
    }

    override fun seekTo(positionMs: Long) {
        calls += "seek:$positionMs"
        this.positionMs = positionMs
    }

    override fun setMuted(muted: Boolean) {
        this.muted = muted
    }

    override fun stop() {
        calls += "stop"
        loaded = null
    }

    override fun release() {
        released = true
    }

    /** What ExoPlayer reports once a video starts playing. */
    fun startPlaying(duration: Long = 60_000) {
        durationMs = duration
        listener!!.onState(EngineState.Ready)
        listener!!.onVideoSize(1280, 720)
        listener!!.onFirstFrame()
        listener!!.onIsPlaying(true)
    }
}

@OptIn(ExperimentalCoroutinesApi::class)
class VideoPlaybackControllerTest {
    private fun video(id: Long, hasStream: Boolean = false) = TimelineItem(
        id = id, collectionId = 1, kind = FileKind.Video, takenAtUs = 0, width = 1920, height = 1080,
        durationMs = 60_000, hasStream = hasStream, title = "clip$id.mp4",
    )

    private class Harness(scope: TestScope, resolve: suspend (TimelineItem) -> VideoSource) {
        val engines = mutableListOf<FakeEngine>()
        var releasedStreams = 0
        val controller = VideoPlaybackController(
            scope = scope.backgroundScope,
            engineFactory = { FakeEngine().also { engines += it } },
            resolve = resolve,
            onReleased = { releasedStreams++ },
        )
        val engine get() = engines.single()
    }

    private val stream = { item: TimelineItem -> VideoSource("http://127.0.0.1:1/t/${item.id}/index.m3u8", isHls = true) }

    @Test
    fun aStreamPlaysThroughTheSharedPlayer() = runTest {
        val h = Harness(this) { stream(it) }
        assertNull(h.controller.engine)
        h.controller.open(video(1, hasStream = true))
        assertEquals(VideoPhase.Resolving, h.controller.state.value.phase)
        runCurrent()
        val s = h.controller.state.value
        assertEquals(VideoPhase.Buffering, s.phase)
        assertTrue(s.isStream)
        assertEquals(listOf("load:http://127.0.0.1:1/t/1/index.m3u8", "play"), h.engine.calls)
        h.engine.startPlaying()
        val playing = h.controller.state.value
        assertEquals(VideoPhase.Ready, playing.phase)
        assertTrue(playing.playing && playing.firstFrame)
        assertEquals(1280 to 720, playing.videoWidth to playing.videoHeight)
        // Opening the same video again (recomposition) does nothing.
        h.controller.open(video(1, hasStream = true))
        assertEquals(2, h.engine.calls.size)
    }

    @Test
    fun oneEngineIsSharedAcrossPagesAndReleasedWithTheViewer() = runTest {
        val h = Harness(this) { VideoSource("file:///cache/${it.id}.bin", isHls = false) }
        h.controller.open(video(1))
        runCurrent()
        h.engine.startPlaying()
        // Swipe: the next page opens, the old one leaves.
        h.controller.open(video(2))
        h.controller.leave(1)
        runCurrent()
        assertEquals(1, h.engines.size)
        assertEquals("file:///cache/2.bin", h.engine.loaded?.uri)
        assertFalse(h.controller.state.value.isStream)
        assertEquals(2L, h.controller.state.value.fileId)
        // Leaving the current page stops it but keeps the engine.
        h.controller.leave(2)
        assertEquals("stop", h.engine.calls.last())
        assertNull(h.controller.state.value.fileId)
        assertFalse(h.engine.released)
        h.controller.release()
        assertTrue(h.engine.released)
        assertEquals(1, h.releasedStreams)
        // Nothing plays after the viewer closed.
        h.controller.open(video(3))
        runCurrent()
        assertNull(h.controller.state.value.fileId)
        h.controller.release()
        assertEquals(1, h.releasedStreams)
    }

    @Test
    fun aPageLeftWhileResolvingNeverLoads() = runTest {
        val gate = CompletableDeferred<Unit>()
        val h = Harness(this) {
            gate.await()
            stream(it)
        }
        h.controller.open(video(1))
        runCurrent()
        h.controller.open(video(2))
        gate.complete(Unit)
        runCurrent()
        assertEquals(listOf("load:http://127.0.0.1:1/t/2/index.m3u8", "play"), h.engine.calls)
    }

    @Test
    fun controlsFadeWhilePlayingAndComeBackOnTap() = runTest {
        val h = Harness(this) { stream(it) }
        h.controller.open(video(1))
        runCurrent()
        h.engine.startPlaying()
        assertTrue(h.controller.state.value.controlsVisible)
        advanceTimeBy(VideoPlaybackController.HIDE_DELAY_MS - 100)
        assertTrue(h.controller.state.value.controlsVisible)
        advanceTimeBy(200)
        assertFalse(h.controller.state.value.controlsVisible)
        h.controller.onSurfaceTap()
        assertTrue(h.controller.state.value.controlsVisible)
        h.controller.onSurfaceTap()
        assertFalse(h.controller.state.value.controlsVisible)
        // Paused: the controls stay.
        h.controller.onSurfaceTap()
        h.controller.togglePlay()
        assertEquals("pause", h.engine.calls.last())
        h.engine.listener!!.onIsPlaying(false)
        advanceTimeBy(VideoPlaybackController.HIDE_DELAY_MS * 3)
        assertTrue(h.controller.state.value.controlsVisible)
        h.controller.togglePlay()
        assertEquals("play", h.engine.calls.last())
    }

    @Test
    fun seekMuteProgressAndReplay() = runTest {
        val h = Harness(this) { stream(it) }
        h.controller.open(video(1))
        assertEquals(60_000L, h.controller.progress.value.durationMs) // from the timeline before the player knows
        runCurrent()
        h.engine.startPlaying(duration = 30_000)
        h.engine.positionMs = 1_000
        advanceTimeBy(VideoPlaybackController.TICK_MS + 1)
        assertEquals(VideoProgress(1_000, 30_000, 0), h.controller.progress.value)
        h.controller.seekTo(45_000)
        assertEquals("seek:30000", h.engine.calls.last())
        assertEquals(30_000L, h.controller.progress.value.positionMs)
        h.controller.seekTo(-5)
        assertEquals("seek:0", h.engine.calls.last())

        // Mute sticks to the shared player across videos.
        h.controller.toggleMute()
        assertTrue(h.engine.muted && h.controller.state.value.muted)
        h.controller.open(video(2))
        runCurrent()
        assertTrue(h.controller.state.value.muted)
        h.controller.toggleMute()
        assertFalse(h.engine.muted)

        // At the end the controls show; play starts over.
        h.engine.startPlaying()
        h.engine.listener!!.onIsPlaying(false)
        h.engine.listener!!.onState(EngineState.Ended)
        val ended = h.controller.state.value
        assertEquals(VideoPhase.Ended, ended.phase)
        assertTrue(ended.controlsVisible && !ended.playing)
        h.controller.togglePlay()
        assertEquals(listOf("seek:0", "play"), h.engine.calls.takeLast(2))
    }

    @Test
    fun errorsAreShownAndTheVideoCanBeRetried() = runTest {
        var fail = true
        val h = Harness(this) {
            if (fail) throw CoreError.Network("offline")
            stream(it)
        }
        h.controller.open(video(1))
        runCurrent()
        val s = h.controller.state.value
        assertEquals(VideoPhase.Error, s.phase)
        assertEquals("offline", s.error)
        assertTrue(h.engines.isEmpty())
        fail = false
        h.controller.open(video(1))
        runCurrent()
        assertEquals(VideoPhase.Buffering, h.controller.state.value.phase)
        h.engine.listener!!.onError("ERROR_CODE_IO_NETWORK_CONNECTION_FAILED")
        assertEquals(VideoPhase.Error, h.controller.state.value.phase)
        assertFalse(h.controller.state.value.playing)
    }

    @Test
    fun timesAreFormatted() {
        assertEquals("0:00", formatTime(0))
        assertEquals("0:09", formatTime(9_999))
        assertEquals("12:05", formatTime(725_000))
        assertEquals("1:02:03", formatTime(3_723_000))
    }
}
