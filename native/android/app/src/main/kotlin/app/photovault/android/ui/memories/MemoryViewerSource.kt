package app.photovault.android.ui.memories

import app.photovault.android.data.CoreRepository
import app.photovault.android.data.TimelineItem

/**
 * Lets the existing photo viewer (ViewerViewModel over a TimelineStore) page through one memory's photos
 * instead of the whole timeline: the timeline calls answer from [items], everything else (download, video
 * streams, thumbnails) goes to the real core. [items] is read on the store's IO dispatcher, so it may look
 * the memory up in core when the process was restored without it.
 */
class MemoryViewerSource(
    private val base: CoreRepository,
    private val items: () -> List<TimelineItem>,
) : CoreRepository by base {
    @Volatile
    private var cached: List<TimelineItem>? = null

    private fun list(): List<TimelineItem> = cached ?: items().also { cached = it }

    override fun timelineCount(): Long = list().size.toLong()

    override fun timelinePage(offset: Long, limit: Int): List<TimelineItem> {
        val all = list()
        if (offset < 0 || offset >= all.size) return emptyList()
        val from = offset.toInt()
        return all.subList(from, minOf(all.size, from + limit)).toList()
    }
}
