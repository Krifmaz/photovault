package app.photovault.android.ui.search

import app.photovault.android.data.CoreError
import app.photovault.android.data.CoreRepository
import app.photovault.android.data.TimelineItem
import app.photovault.android.data.ml.MlRepository
import app.photovault.android.data.ml.SearchKind
import app.photovault.android.ui.timeline.DEFAULT_ZOOM_INDEX
import app.photovault.android.ui.timeline.GridEntry
import app.photovault.android.ui.timeline.GridModel
import app.photovault.android.ui.timeline.Grouping
import app.photovault.android.ui.timeline.TimelineLabels
import app.photovault.android.ui.timeline.buildGrid
import kotlinx.coroutines.runBlocking

/*
 * Lists of photos that are not the timeline (search results, a person's photos), shown in the timeline's grid
 * cells and opened in the timeline's viewer.
 */

/** A route-safe name for where a photo list came from, so the viewer can find (or reload) the same list. */
object PhotoSource {
    fun search(kind: SearchKind, key: String): String = "s:${kind.name}:$key"
    fun person(personId: String): String = "p:$personId"
}

/**
 * The photo list the user last opened, so the viewer pages through exactly what the grid showed. After process
 * death it is empty and [load] asks core again.
 */
object ResultLists {
    @Volatile
    private var last: Pair<String, List<TimelineItem>>? = null

    fun put(source: String, items: List<TimelineItem>) {
        last = source to items
    }

    /** Forgets the list (sign-out). */
    fun clear() {
        last = null
    }

    /** The list for [source]. Blocking: call it off the main thread (the viewer's store reads it on IO). */
    fun load(ml: MlRepository, source: String): List<TimelineItem> {
        last?.let { (s, items) -> if (s == source) return items }
        val items = try {
            when {
                source.startsWith("p:") -> ml.personFiles(source.removePrefix("p:"))
                source.startsWith("s:") -> {
                    val rest = source.removePrefix("s:")
                    val kind = SearchKind.entries.firstOrNull { it.name == rest.substringBefore(':') }
                    if (kind == null) emptyList() else runBlocking { ml.searchResults(kind, rest.substringAfter(':')) }
                }
                else -> emptyList()
            }
        } catch (e: CoreError) {
            emptyList()
        }
        put(source, items)
        return items
    }
}

/**
 * A [CoreRepository] whose "timeline" is a fixed list (loaded on first use, off the main thread), so the
 * timeline's [app.photovault.android.data.TimelineStore] and viewer work unchanged over search results.
 * Everything else goes to [base].
 */
class ListCore(base: CoreRepository, load: () -> List<TimelineItem>) : CoreRepository by base {
    private val items: List<TimelineItem> by lazy(load)

    override fun timelineCount(): Long = items.size.toLong()

    override fun timelinePage(offset: Long, limit: Int): List<TimelineItem> {
        if (offset < 0 || offset >= items.size || limit <= 0) return emptyList()
        val from = offset.toInt()
        return items.subList(from, minOf(items.size, from + limit)).toList()
    }
}

/**
 * The grid for a photo list at the timeline's default density: under month headers ([grouped], for lists in
 * date order) or flat (magic search, best match first). Pure; runs off the main thread.
 */
fun photoGrid(items: List<TimelineItem>, grouped: Boolean, labels: TimelineLabels): GridModel {
    if (grouped) {
        val (entries, map) = buildGrid(items, Grouping.Month, labels)
        return GridModel(entries, DEFAULT_ZOOM_INDEX, map)
    }
    val entries = ArrayList<GridEntry>(items.size)
    items.forEachIndexed { i, item -> entries += GridEntry.Cell(item, i) }
    return GridModel(entries, DEFAULT_ZOOM_INDEX, IntArray(items.size) { it })
}
