package app.photovault.android.widget

import app.photovault.android.data.CoreRepository
import app.photovault.android.data.FileKind
import app.photovault.android.data.TimelineItem
import java.time.Instant
import java.time.ZoneOffset

/** What a widget shows: one photo and a caption, or a message when there is none. */
sealed interface WidgetPick {
    data class Photo(val item: TimelineItem, val caption: Caption) : WidgetPick
    data object SignedOut : WidgetPick
    data object Empty : WidgetPick
    data object AlbumMissing : WidgetPick
}

/** How the caption reads; strings are resolved by the provider. */
sealed interface Caption {
    data class YearsAgo(val years: Int) : Caption
    data object Recent : Caption
    data class Album(val name: String) : Caption
}

/**
 * Picks the widgets' photos from core's local library (no network: only the thumbnail fetch may go online).
 * Each refresh moves on to the next photo of the set ([slot] advances every [WidgetRefresh.PERIOD_MS]).
 */
object WidgetPicks {
    private const val SET = 30

    /** "On this day" in earlier years; without any, a recent photo. Videos only when nothing else is there. */
    fun memories(core: CoreRepository, nowMs: Long, utcOffsetSecs: Int, slot: Long): WidgetPick {
        if (core.currentAccount() == null) return WidgetPick.SignedOut
        val today = Instant.ofEpochMilli(nowMs).atOffset(ZoneOffset.ofTotalSeconds(utcOffsetSecs)).year
        val memories = preferStills(core.onThisDay(nowMs * 1000, utcOffsetSecs, SET))
        if (memories.isNotEmpty()) {
            val item = memories[Math.floorMod(slot, memories.size.toLong()).toInt()]
            val year = Instant.ofEpochMilli(item.takenAtUs / 1000).atOffset(ZoneOffset.ofTotalSeconds(utcOffsetSecs)).year
            return WidgetPick.Photo(item, Caption.YearsAgo((today - year).coerceAtLeast(1)))
        }
        val recent = preferStills(core.timelinePage(0, SET))
        if (recent.isEmpty()) return WidgetPick.Empty
        return WidgetPick.Photo(recent[Math.floorMod(slot, recent.size.toLong()).toInt()], Caption.Recent)
    }

    /** A photo of the chosen album (only while the album is visible: not hidden, not deleted). */
    fun album(core: CoreRepository, albumId: Long, slot: Long): WidgetPick {
        if (core.currentAccount() == null) return WidgetPick.SignedOut
        val album = core.collections().firstOrNull { it.id == albumId } ?: return WidgetPick.AlbumMissing
        val items = preferStills(core.collectionPage(albumId, 0, SET * 2))
        if (items.isEmpty()) return WidgetPick.Empty
        return WidgetPick.Photo(items[Math.floorMod(slot, items.size.toLong()).toInt()], Caption.Album(album.name))
    }

    private fun preferStills(items: List<TimelineItem>): List<TimelineItem> {
        val stills = items.filter { it.kind == FileKind.Image || it.kind == FileKind.LivePhoto }
        return stills.ifEmpty { items }
    }
}
