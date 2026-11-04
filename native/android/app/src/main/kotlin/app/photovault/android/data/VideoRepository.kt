package app.photovault.android.data

/**
 * Streamable videos (contract v1.2 "video"): Ente's `vid_preview` streams, decrypted by core and served to
 * the player from core's loopback server. Failures are thrown as [CoreError].
 */
interface VideoRepository {
    /**
     * The stream of a video, or null when it has none (play the original instead). Core answers from its
     * local status for the account's own videos and asks the server for videos shared by others, so this may
     * be called for every video.
     */
    suspend fun videoStream(fileId: Long): VideoStream?

    /**
     * A viewer opened and may play streams; it calls [stopStreams] when it closes. Several viewers can be
     * alive at once (an album viewer over the timeline's, or one still leaving while the next opens), so
     * core's streams end only when the last one closed.
     */
    fun viewerOpened() {}

    /** A viewer closed: ends every stream (their loopback URLs stop working) once no other viewer is open. */
    fun stopStreams()
}

/** `VideoStream` record: [playlistUrl] is an HLS playlist on 127.0.0.1 served by core. */
data class VideoStream(
    val playlistUrl: String,
    val width: Int,
    val height: Int,
    val durationMs: Long,
    val sizeBytes: Long,
)

/**
 * A [VideoRepository] on top of contract v1's `stream_playlist` only (FakeCore, UnavailableCore, or any core
 * without the v1.2 exports): no size or duration, nothing to stop.
 */
class PlaylistVideoRepository(private val core: CoreRepository) : VideoRepository {
    override suspend fun videoStream(fileId: Long): VideoStream? =
        core.streamPlaylist(fileId)?.let { VideoStream(it, 0, 0, 0, 0) }

    override fun stopStreams() = Unit
}
