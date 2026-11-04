package app.photovault.android.ui.viewer.video

import app.photovault.android.data.CoreError
import app.photovault.android.data.TimelineItem
import app.photovault.android.ui.viewer.VideoSource
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import java.io.IOException

/** Where the current video is. */
enum class VideoPhase { Idle, Resolving, Buffering, Ready, Ended, Error }

/** What the viewer's video page shows. Positions are in [VideoProgress], read only by the seek bar. */
data class VideoUiState(
    val fileId: Long? = null,
    val phase: VideoPhase = VideoPhase.Idle,
    val playing: Boolean = false,
    val muted: Boolean = false,
    /** Playing core's stream (Ente's HLS preview), not the downloaded original. */
    val isStream: Boolean = false,
    val controlsVisible: Boolean = true,
    /** The first frame is on screen: the thumbnail under the surface can go. */
    val firstFrame: Boolean = false,
    val videoWidth: Int = 0,
    val videoHeight: Int = 0,
    val error: String? = null,
)

data class VideoProgress(val positionMs: Long = 0, val durationMs: Long = 0, val bufferedMs: Long = 0)

/**
 * The viewer's one player, shared by all pages: the settled page [open]s its video, a page that leaves calls
 * [leave], and [release] frees the player when the viewer closes. The engine is created on the first video.
 *
 * [resolve] picks the source (core's stream, else the downloaded original). Controls fade out [hideDelayMs]
 * after the last interaction while playing; [progress] ticks every [tickMs] only while the controls are
 * visible and the video plays, so the rest of the page does not recompose with the position.
 *
 * Main thread only (the engine is a Media3 player); [scope] is the view model's.
 */
class VideoPlaybackController(
    private val scope: CoroutineScope,
    private val engineFactory: () -> PlaybackEngine,
    private val resolve: suspend (TimelineItem) -> VideoSource,
    private val onReleased: () -> Unit = {},
    private val hideDelayMs: Long = HIDE_DELAY_MS,
    private val tickMs: Long = TICK_MS,
) {
    private val _state = MutableStateFlow(VideoUiState())
    val state: StateFlow<VideoUiState> = _state.asStateFlow()

    private val _progress = MutableStateFlow(VideoProgress())
    val progress: StateFlow<VideoProgress> = _progress.asStateFlow()

    private var engineOrNull: PlaybackEngine? = null
    private var released = false
    private var loadJob: Job? = null
    private var hideJob: Job? = null
    private var tickJob: Job? = null

    /** The engine, once a video was opened (the surface attaches to it). */
    val engine: PlaybackEngine? get() = engineOrNull

    private val listener = object : PlaybackEngine.Listener {
        override fun onState(state: EngineState) {
            _state.update {
                if (it.phase == VideoPhase.Error || it.phase == VideoPhase.Resolving || it.fileId == null) return@update it
                when (state) {
                    EngineState.Buffering -> it.copy(phase = VideoPhase.Buffering)
                    EngineState.Ready -> it.copy(phase = VideoPhase.Ready)
                    EngineState.Ended -> it.copy(phase = VideoPhase.Ended, playing = false, controlsVisible = true)
                    EngineState.Idle -> it
                }
            }
            if (state == EngineState.Ended) {
                hideJob?.cancel()
                refreshProgress()
            }
            syncTicker()
        }

        override fun onIsPlaying(playing: Boolean) {
            _state.update { it.copy(playing = playing) }
            if (playing) scheduleHide() else hideJob?.cancel()
            syncTicker()
        }

        override fun onVideoSize(width: Int, height: Int) {
            _state.update { it.copy(videoWidth = width, videoHeight = height) }
        }

        override fun onFirstFrame() {
            _state.update { it.copy(firstFrame = true) }
        }

        override fun onError(message: String) {
            _state.update { it.copy(phase = VideoPhase.Error, playing = false, error = message, controlsVisible = true) }
            syncTicker()
        }
    }

    private fun engine(): PlaybackEngine = engineOrNull ?: engineFactory().also {
        it.setListener(listener)
        it.setMuted(_state.value.muted)
        engineOrNull = it
    }

    /** The pager settled on [item]: play it (a no-op if it is already the current video). */
    fun open(item: TimelineItem) {
        if (released) return
        val cur = _state.value
        if (cur.fileId == item.id && cur.phase != VideoPhase.Idle && cur.phase != VideoPhase.Error) return
        loadJob?.cancel()
        hideJob?.cancel()
        engineOrNull?.stop()
        _state.value = VideoUiState(fileId = item.id, phase = VideoPhase.Resolving, muted = cur.muted)
        _progress.value = VideoProgress(durationMs = item.durationMs?.toLong() ?: 0L)
        syncTicker()
        loadJob = scope.launch {
            val source = try {
                resolve(item)
            } catch (e: CancellationException) {
                throw e
            } catch (e: CoreError) {
                fail(item.id, e.message)
                return@launch
            } catch (e: IOException) {
                fail(item.id, e.message)
                return@launch
            }
            if (_state.value.fileId != item.id || released) return@launch
            _state.update { it.copy(phase = VideoPhase.Buffering, isStream = source.isHls) }
            val e = engine()
            e.load(source)
            e.play()
        }
    }

    private fun fail(fileId: Long, message: String?) {
        _state.update {
            if (it.fileId != fileId) it else it.copy(phase = VideoPhase.Error, error = message.orEmpty(), controlsVisible = true)
        }
    }

    /** The page of [fileId] is no longer the settled one: stop it. */
    fun leave(fileId: Long) {
        if (_state.value.fileId != fileId) return
        loadJob?.cancel()
        hideJob?.cancel()
        engineOrNull?.stop()
        _state.update { VideoUiState(muted = it.muted) }
        _progress.value = VideoProgress()
        syncTicker()
    }

    fun togglePlay() {
        val e = engineOrNull ?: return
        val s = _state.value
        when {
            s.phase == VideoPhase.Ended -> {
                e.seekTo(0)
                e.play()
            }
            s.playing -> e.pause()
            s.phase == VideoPhase.Buffering || s.phase == VideoPhase.Ready -> e.play()
        }
        showControls()
    }

    /** Seek to [positionMs] (clamped to the duration when known). */
    fun seekTo(positionMs: Long) {
        val e = engineOrNull ?: return
        val d = e.durationMs.takeIf { it > 0 } ?: _progress.value.durationMs
        val target = if (d > 0) positionMs.coerceIn(0, d) else positionMs.coerceAtLeast(0)
        e.seekTo(target)
        if (_state.value.phase == VideoPhase.Ended) _state.update { it.copy(phase = VideoPhase.Buffering) }
        _progress.update { it.copy(positionMs = target) }
        showControls()
    }

    fun toggleMute() {
        val muted = !_state.value.muted
        _state.update { it.copy(muted = muted) }
        engineOrNull?.setMuted(muted)
        showControls()
    }

    /** A tap on the video: show or hide the controls. */
    fun onSurfaceTap() {
        if (_state.value.controlsVisible && _state.value.playing) {
            hideJob?.cancel()
            _state.update { it.copy(controlsVisible = false) }
            syncTicker()
        } else {
            showControls()
        }
    }

    private fun showControls() {
        _state.update { it.copy(controlsVisible = true) }
        refreshProgress()
        scheduleHide()
        syncTicker()
    }

    private fun scheduleHide() {
        hideJob?.cancel()
        if (!_state.value.playing) return
        hideJob = scope.launch {
            delay(hideDelayMs)
            if (_state.value.playing) {
                _state.update { it.copy(controlsVisible = false) }
                syncTicker()
            }
        }
    }

    private fun refreshProgress() {
        val e = engineOrNull ?: return
        val d = e.durationMs.takeIf { it > 0 } ?: _progress.value.durationMs
        _progress.value = VideoProgress(e.positionMs, d, e.bufferedMs)
    }

    private fun syncTicker() {
        val s = _state.value
        val want = s.controlsVisible && s.playing && !released
        if (want && tickJob?.isActive != true) {
            tickJob = scope.launch {
                while (isActive) {
                    refreshProgress()
                    delay(tickMs)
                }
            }
        } else if (!want) {
            tickJob?.cancel()
            tickJob = null
        }
    }

    /** The viewer closed: free the player and end core's streams. */
    fun release() {
        if (released) return
        released = true
        loadJob?.cancel()
        hideJob?.cancel()
        tickJob?.cancel()
        engineOrNull?.release()
        engineOrNull = null
        _state.value = VideoUiState()
        onReleased()
    }

    companion object {
        const val HIDE_DELAY_MS = 3_000L
        const val TICK_MS = 250L
    }
}
