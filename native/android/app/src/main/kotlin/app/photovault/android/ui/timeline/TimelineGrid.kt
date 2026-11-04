package app.photovault.android.ui.timeline

import androidx.compose.runtime.Immutable
import app.photovault.android.data.TimelineItem
import java.time.Instant
import java.time.LocalDate
import java.time.ZoneId
import java.time.format.DateTimeFormatter
import java.util.Locale

/** How the grid groups photos under its sticky headers. */
enum class Grouping { Day, Month, Year }

/** One pinch step: the column count and the grouping it uses. */
data class ZoomLevel(val columns: Int, val grouping: Grouping)

/** Pinching in goes up the list (bigger cells), pinching out goes down. */
val ZoomLevels: List<ZoomLevel> = listOf(
    ZoomLevel(2, Grouping.Day),
    ZoomLevel(3, Grouping.Day),
    ZoomLevel(4, Grouping.Day),
    ZoomLevel(6, Grouping.Month),
    ZoomLevel(10, Grouping.Year),
)
const val DEFAULT_ZOOM_INDEX = 2

/** One slot of the grid: a full-width header or a photo cell. */
@Immutable
sealed interface GridEntry {
    val key: String

    @Immutable
    data class Header(override val key: String, val label: String, val count: Int) : GridEntry

    @Immutable
    data class Cell(val item: TimelineItem, val timelineIndex: Int) : GridEntry {
        override val key: String get() = "p${item.id}"
    }
}

/** Content types for LazyVerticalGrid, so headers and cells are recycled separately. */
object GridContentType {
    const val HEADER = 0
    const val CELL = 1

    /** The memories strip, the grid's first full-span item when there are memories (before entry 0). */
    const val MEMORIES = 2
}

/** The grid at one zoom level. [entryIndexOfTimeline] maps a timeline index to its entry index. */
@Immutable
data class GridModel(
    val entries: List<GridEntry>,
    val zoomIndex: Int,
    val entryIndexOfTimeline: IntArray,
) {
    val columns: Int get() = ZoomLevels[zoomIndex].columns

    override fun equals(other: Any?): Boolean =
        other is GridModel && other.entries === entries && other.zoomIndex == zoomIndex

    override fun hashCode(): Int = System.identityHashCode(entries) * 31 + zoomIndex

    companion object {
        val Empty = GridModel(emptyList(), DEFAULT_ZOOM_INDEX, IntArray(0))
    }
}

/** Header wording. [today] / [yesterday] come from string resources. */
class TimelineLabels(
    private val today: String,
    private val yesterday: String,
    private val zone: ZoneId = ZoneId.systemDefault(),
    private val locale: Locale = Locale.getDefault(),
    private val clock: () -> LocalDate = { LocalDate.now(zone) },
) {
    private val dayThisYear = DateTimeFormatter.ofPattern("EEE, d MMM", locale)
    private val dayOtherYear = DateTimeFormatter.ofPattern("EEE, d MMM yyyy", locale)
    private val month = DateTimeFormatter.ofPattern("LLLL yyyy", locale)

    fun dateOf(takenAtUs: Long): LocalDate =
        Instant.ofEpochSecond(Math.floorDiv(takenAtUs, 1_000_000L), Math.floorMod(takenAtUs, 1_000_000L) * 1000L)
            .atZone(zone).toLocalDate()

    fun label(date: LocalDate, grouping: Grouping): String {
        val now = clock()
        return when (grouping) {
            Grouping.Day -> when {
                date == now -> today
                date == now.minusDays(1) -> yesterday
                date.year == now.year -> dayThisYear.format(date)
                else -> dayOtherYear.format(date)
            }
            Grouping.Month -> month.format(date).replaceFirstChar { it.titlecase(locale) }
            Grouping.Year -> date.year.toString()
        }
    }

    fun sectionKey(date: LocalDate, grouping: Grouping): String = when (grouping) {
        Grouping.Day -> "d$date"
        Grouping.Month -> "m${date.year}-${date.monthValue}"
        Grouping.Year -> "y${date.year}"
    }

    fun sameSection(a: LocalDate, b: LocalDate, grouping: Grouping): Boolean = when (grouping) {
        Grouping.Day -> a == b
        Grouping.Month -> a.year == b.year && a.monthValue == b.monthValue
        Grouping.Year -> a.year == b.year
    }
}

/** Builds the header + cell list for [items] (newest first). Pure; runs off the main thread. */
fun buildGrid(items: List<TimelineItem>, grouping: Grouping, labels: TimelineLabels): Pair<List<GridEntry>, IntArray> {
    val out = ArrayList<GridEntry>(items.size + items.size / 8 + 1)
    val map = IntArray(items.size)
    var headerAt = -1
    var sectionDate: LocalDate? = null
    var count = 0
    fun closeSection() {
        if (headerAt >= 0) {
            val h = out[headerAt] as GridEntry.Header
            out[headerAt] = h.copy(count = count)
        }
    }
    for ((i, item) in items.withIndex()) {
        val date = labels.dateOf(item.takenAtUs)
        val current = sectionDate
        if (current == null || !labels.sameSection(current, date, grouping)) {
            closeSection()
            sectionDate = date
            count = 0
            headerAt = out.size
            out += GridEntry.Header(labels.sectionKey(date, grouping), labels.label(date, grouping), 0)
        }
        map[i] = out.size
        out += GridEntry.Cell(item, i)
        count++
    }
    closeSection()
    return out to map
}
