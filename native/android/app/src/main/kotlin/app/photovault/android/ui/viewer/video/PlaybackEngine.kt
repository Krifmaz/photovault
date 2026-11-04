package app.photovault.android.ui.viewer.video

import app.photovault.android.ui.viewer.VideoSource

/**
 * The player behind the viewer's videos, as [VideoPlaybackController] needs it. [ExoPlaybackEngine] is the real
 * one (Media3 ExoPlayer); tests use a fake. Called on the main thread only.
 */
interface PlaybackEngine {
    /** Receives the engine's events (on the main thread). */
    interface Listener {
        fun onState(state: EngineState)
        fun onIsPlaying(playing: Boolean)
        fun onVideoSize(width: Int, height: Int)
        fun onFirstFrame()
        fun onError(message: String)
    }

    fun setListener(listener: Listener?)

    /** Prepare [source] (HLS when [VideoSource.isHls]) from the start; replaces what was loaded. */
    fun load(source: VideoSource)
    fun play()
    fun pause()
    fun seekTo(positionMs: Long)
    fun setMuted(muted: Boolean)

    val positionMs: Long
    /** Duration, or 0 while unknown. */
    val durationMs: Long
    val bufferedMs: Long

    /** Stop and drop the loaded media (keeps the engine for the next video). */
    fun stop()

    /** Free the decoder and buffers; the engine is unusable afterwards. */
    fun release()
}

enum class EngineState { Idle, Buffering, Ready, Ended }
