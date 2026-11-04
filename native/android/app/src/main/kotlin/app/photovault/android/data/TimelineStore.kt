package app.photovault.android.data

import kotlinx.coroutines.CoroutineDispatcher
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withContext

/**
 * The timeline as loaded so far, shared by the grid and the viewer: pages of [CHUNK] from core's
 * `timeline_page` (local store only, newest first), appended on demand as the user scrolls.
 */
class TimelineStore(
    private val core: CoreRepository,
    private val io: CoroutineDispatcher = Dispatchers.IO,
) {
    private val mutex = Mutex()
    private val _items = MutableStateFlow<List<TimelineItem>>(emptyList())
    private val _total = MutableStateFlow(0L)
    private val _loaded = MutableStateFlow(false)

    /** Items loaded so far, newest first. */
    val items: StateFlow<List<TimelineItem>> = _items.asStateFlow()

    /** core's `timeline_count` at the last refresh. */
    val total: StateFlow<Long> = _total.asStateFlow()

    /** True once the first refresh finished (so an empty list means an empty library). */
    val loaded: StateFlow<Boolean> = _loaded.asStateFlow()

    /** Reloads from the start (after a sync), keeping at least as many items as were loaded. */
    suspend fun refresh() = mutex.withLock { reloadLocked(maxOf(_items.value.size, CHUNK)) }

    /**
     * Loads the first [keep] items. Core swaps its index at the end of a sync, possibly between two of
     * these pages, so the count is read before and after: when it moved, the read is repeated.
     */
    private suspend fun reloadLocked(keep: Int) {
        var attempt = 0
        while (true) {
            val (stable, count, fresh) = withContext(io) {
                val count = core.timelineCount()
                val out = ArrayList<TimelineItem>(minOf(count, keep.toLong()).toInt())
                while (out.size < keep && out.size < count) {
                    val page = core.timelinePage(out.size.toLong(), CHUNK)
                    if (page.isEmpty()) break
                    out += page
                }
                Triple(core.timelineCount() == count, count, out)
            }
            if (stable || ++attempt >= MAX_RELOADS) {
                _total.value = count
                _items.value = fresh.distinctById()
                _loaded.value = true
                return
            }
        }
    }

    /**
     * Makes sure items up to [index] (exclusive) are loaded, a chunk at a time. Pages are fetched by offset,
     * so a page is only appended while core's index is the one the list was loaded from (same count before
     * and after the page); otherwise the list is reloaded from the start instead of mixing two indexes.
     * Ids are de-duplicated before publishing either way: the grid and the pager key by id.
     */
    suspend fun ensureLoaded(index: Int) {
        if (index <= _items.value.size || _items.value.size >= _total.value) return
        mutex.withLock {
            while (_items.value.size < index && _items.value.size < _total.value) {
                val current = _items.value
                val expected = _total.value
                val (before, page, after) = withContext(io) {
                    Triple(core.timelineCount(), core.timelinePage(current.size.toLong(), CHUNK), core.timelineCount())
                }
                if (before != expected || after != expected) {
                    reloadLocked(maxOf(index, current.size))
                    return@withLock
                }
                if (page.isEmpty()) {
                    _total.value = current.size.toLong()
                    break
                }
                val next = (current + page).distinctById()
                _items.value = next
                // Only duplicates (the index changed in a way the count does not show): stop, the next
                // refresh after the sync reloads it.
                if (next.size == current.size) break
            }
        }
    }

    /** Forgets everything (sign-out, server switch). */
    suspend fun clear() = mutex.withLock {
        _items.value = emptyList()
        _total.value = 0
        _loaded.value = false
    }

    companion object {
        const val CHUNK = 200

        /** How far ahead of the last visible item to keep loaded. */
        const val AHEAD = 120

        private const val MAX_RELOADS = 3

        private fun List<TimelineItem>.distinctById(): List<TimelineItem> {
            val seen = HashSet<Long>(size * 2)
            return if (all { seen.add(it.id) }) this else distinctBy { it.id }
        }
    }
}
