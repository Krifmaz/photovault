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
    suspend fun refresh() = mutex.withLock {
        val keep = maxOf(_items.value.size, CHUNK)
        val (count, fresh) = withContext(io) {
            val count = core.timelineCount()
            val out = ArrayList<TimelineItem>(minOf(count, keep.toLong()).toInt())
            while (out.size < keep && out.size < count) {
                val page = core.timelinePage(out.size.toLong(), CHUNK)
                if (page.isEmpty()) break
                out += page
            }
            count to out
        }
        _total.value = count
        _items.value = fresh
        _loaded.value = true
    }

    /** Makes sure items up to [index] (exclusive) are loaded, a chunk at a time. */
    suspend fun ensureLoaded(index: Int) {
        if (index <= _items.value.size || _items.value.size >= _total.value) return
        mutex.withLock {
            while (_items.value.size < index && _items.value.size < _total.value) {
                val current = _items.value
                val page = withContext(io) { core.timelinePage(current.size.toLong(), CHUNK) }
                if (page.isEmpty()) {
                    _total.value = current.size.toLong()
                    break
                }
                _items.value = current + page
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
    }
}
