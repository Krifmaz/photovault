package app.photovault.android.data.fake

import app.photovault.android.data.CoreError
import app.photovault.android.data.VideoRepository
import app.photovault.android.data.VideoStream

/**
 * In-memory [VideoRepository] for tests: [streams] maps file ids to their stream, [failing] file ids throw a
 * network error. Records calls so tests can check what the viewer asked for.
 */
class FakeVideoRepository(
    private val streams: Map<Long, VideoStream> = emptyMap(),
    private val failing: Set<Long> = emptySet(),
) : VideoRepository {
    val requested = mutableListOf<Long>()
    var stopCount = 0
        private set
    var openCount = 0
        private set

    override suspend fun videoStream(fileId: Long): VideoStream? {
        requested += fileId
        if (fileId in failing) throw CoreError.Network("stream unavailable")
        return streams[fileId]
    }

    override fun viewerOpened() {
        openCount++
    }

    override fun stopStreams() {
        stopCount++
    }
}
