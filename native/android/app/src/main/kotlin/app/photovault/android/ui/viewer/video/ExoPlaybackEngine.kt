package app.photovault.android.ui.viewer.video

import android.content.Context
import androidx.media3.common.AudioAttributes
import androidx.media3.common.C
import androidx.media3.common.MediaItem
import androidx.media3.common.MimeTypes
import androidx.media3.common.PlaybackException
import androidx.media3.common.Player
import androidx.media3.common.VideoSize
import androidx.media3.exoplayer.ExoPlayer
import app.photovault.android.ui.viewer.VideoSource

/**
 * [PlaybackEngine] over one Media3 ExoPlayer. Streams are core's loopback HLS playlists (the HLS module plays
 * them); originals are `file://` URIs of the decrypted download. The player takes audio focus while it plays
 * (and pauses when headphones are unplugged), and is released with the viewer.
 */
class ExoPlaybackEngine(context: Context) : PlaybackEngine {
    val player: ExoPlayer = ExoPlayer.Builder(context.applicationContext)
        .setAudioAttributes(
            AudioAttributes.Builder()
                .setUsage(C.USAGE_MEDIA)
                .setContentType(C.AUDIO_CONTENT_TYPE_MOVIE)
                .build(),
            /* handleAudioFocus = */ true,
        )
        .setHandleAudioBecomingNoisy(true)
        .build()
        .apply { repeatMode = Player.REPEAT_MODE_OFF }

    private var listener: PlaybackEngine.Listener? = null

    private val events = object : Player.Listener {
        override fun onPlaybackStateChanged(playbackState: Int) {
            listener?.onState(
                when (playbackState) {
                    Player.STATE_BUFFERING -> EngineState.Buffering
                    Player.STATE_READY -> EngineState.Ready
                    Player.STATE_ENDED -> EngineState.Ended
                    else -> EngineState.Idle
                },
            )
        }

        override fun onIsPlayingChanged(isPlaying: Boolean) {
            listener?.onIsPlaying(isPlaying)
        }

        override fun onVideoSizeChanged(videoSize: VideoSize) {
            if (videoSize.width > 0 && videoSize.height > 0) {
                val w = (videoSize.width * videoSize.pixelWidthHeightRatio).toInt()
                listener?.onVideoSize(w, videoSize.height)
            }
        }

        override fun onRenderedFirstFrame() {
            listener?.onFirstFrame()
        }

        override fun onPlayerError(error: PlaybackException) {
            listener?.onError(error.errorCodeName)
        }
    }

    init {
        player.addListener(events)
    }

    override fun setListener(listener: PlaybackEngine.Listener?) {
        this.listener = listener
    }

    override fun load(source: VideoSource) {
        val item = MediaItem.Builder()
            .setUri(source.uri)
            .apply { if (source.isHls) setMimeType(MimeTypes.APPLICATION_M3U8) }
            .build()
        player.setMediaItem(item, /* startPositionMs = */ 0L)
        player.prepare()
    }

    override fun play() {
        player.play()
    }

    override fun pause() {
        player.pause()
    }

    override fun seekTo(positionMs: Long) {
        player.seekTo(positionMs)
    }

    override fun setMuted(muted: Boolean) {
        player.volume = if (muted) 0f else 1f
    }

    override val positionMs: Long get() = player.currentPosition
    override val durationMs: Long get() = player.duration.takeIf { it != C.TIME_UNSET && it > 0 } ?: 0L
    override val bufferedMs: Long get() = player.bufferedPosition

    override fun stop() {
        player.stop()
        player.clearMediaItems()
    }

    override fun release() {
        player.removeListener(events)
        listener = null
        player.release()
    }
}
