package app.photovault.android.data

import app.photovault.core.Core
import app.photovault.core.CoreException
import java.util.concurrent.atomic.AtomicInteger

/** [VideoRepository] over photovault-core's v1.2 exports (`video_stream`, `stop_streams`). */
internal class RealVideoRepository(private val core: Core) : VideoRepository {
    /** Open viewers; core's `stop_streams` ends the streams of all of them, so only the last one calls it. */
    private val viewers = AtomicInteger(0)

    override suspend fun videoStream(fileId: Long): VideoStream? = try {
        core.videoStream(fileId)?.let {
            VideoStream(it.playlistUrl, it.width.toInt(), it.height.toInt(), it.durationMs.toLong(), it.sizeBytes.toLong())
        }
    } catch (e: CoreException) {
        throw e.toApp()
    }

    override fun viewerOpened() {
        viewers.incrementAndGet()
    }

    override fun stopStreams() {
        val left = viewers.updateAndGet { (it - 1).coerceAtLeast(0) }
        if (left == 0) core.stopStreams()
    }
}
