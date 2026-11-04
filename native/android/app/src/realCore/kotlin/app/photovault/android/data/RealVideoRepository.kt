package app.photovault.android.data

import app.photovault.core.Core
import app.photovault.core.CoreException

/** [VideoRepository] over photovault-core's v1.2 exports (`video_stream`, `stop_streams`). */
internal class RealVideoRepository(private val core: Core) : VideoRepository {
    override suspend fun videoStream(fileId: Long): VideoStream? = try {
        core.videoStream(fileId)?.let {
            VideoStream(it.playlistUrl, it.width.toInt(), it.height.toInt(), it.durationMs.toLong(), it.sizeBytes.toLong())
        }
    } catch (e: CoreException) {
        throw e.toApp()
    }

    override fun stopStreams() = core.stopStreams()
}
