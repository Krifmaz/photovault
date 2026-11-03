package app.photovault.android.ui.timeline

import app.photovault.android.data.FileKind
import app.photovault.android.data.TimelineItem
import org.junit.Assert.assertEquals
import org.junit.Test
import java.time.LocalDate
import java.time.LocalDateTime
import java.time.ZoneOffset
import java.util.Locale

class GridBuilderTest {
    private val labels = TimelineLabels(
        today = "Today",
        yesterday = "Yesterday",
        zone = ZoneOffset.UTC,
        locale = Locale.US,
        clock = { LocalDate.of(2026, 11, 3) },
    )

    private fun item(id: Long, y: Int, m: Int, d: Int, h: Int = 12) = TimelineItem(
        id = id, collectionId = 1, kind = FileKind.Image,
        takenAtUs = LocalDateTime.of(y, m, d, h, 0).toEpochSecond(ZoneOffset.UTC) * 1_000_000L,
        width = 4, height = 3, durationMs = null, hasStream = false, title = "x",
    )

    private val items = listOf(
        item(1, 2026, 11, 3, 18), item(2, 2026, 11, 3, 9),
        item(3, 2026, 11, 2),
        item(4, 2026, 10, 30),
        item(5, 2025, 12, 31), item(6, 2025, 1, 1),
    )

    @Test
    fun dayGroupingHasOneHeaderPerDay() {
        val (entries, map) = buildGrid(items, Grouping.Day, labels)
        val headers = entries.filterIsInstance<GridEntry.Header>()
        assertEquals(listOf("Today", "Yesterday", "Fri, 30 Oct", "Wed, 31 Dec 2025", "Wed, 1 Jan 2025"), headers.map { it.label })
        assertEquals(listOf(2, 1, 1, 1, 1), headers.map { it.count })
        assertEquals(items.size + headers.size, entries.size)
        // every timeline index points at its own cell
        items.indices.forEach { assertEquals(items[it].id, (entries[map[it]] as GridEntry.Cell).item.id) }
    }

    @Test
    fun monthAndYearGrouping() {
        val months = buildGrid(items, Grouping.Month, labels).first.filterIsInstance<GridEntry.Header>()
        assertEquals(listOf("November 2026", "October 2026", "December 2025", "January 2025"), months.map { it.label })
        val years = buildGrid(items, Grouping.Year, labels).first.filterIsInstance<GridEntry.Header>()
        assertEquals(listOf("2026", "2025"), years.map { it.label })
        assertEquals(listOf(4, 2), years.map { it.count })
    }

    @Test
    fun keysAreUnique() {
        for (g in Grouping.entries) {
            val keys = buildGrid(items, g, labels).first.map { it.key }
            assertEquals(keys.size, keys.toSet().size)
        }
    }

    @Test
    fun durationFormat() {
        assertEquals("0:07", formatDuration(7_400))
        assertEquals("1:02:03", formatDuration(3_723_000))
    }
}
