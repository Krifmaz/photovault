package app.photovault.android.ui.memories

import app.photovault.android.AppGraph
import app.photovault.android.data.CoreError
import app.photovault.android.data.ml.Memory
import app.photovault.android.data.ml.MlRepository
import kotlinx.coroutines.CoroutineDispatcher
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withContext
import java.time.Instant
import java.time.ZoneId

/**
 * Today's memories (core's `memories(now_us, utc_offset_secs)`), shared by the strip above the timeline
 * and the story viewer so that marking one seen dims its cover at once.
 *
 * Core computes them from the local index only; the call can still take a while the first time on a day
 * (it builds the day's cache), so it always runs on [io], never on the caller's thread. Unseen memories
 * come first, otherwise core's order is kept (Ente's memories strip sorts the same way, stable).
 *
 * Based on Ente (ente.com): mobile/apps/photos/lib/ui/home/memories/memories_strip.dart.
 */
class MemoryFeed(
    private val ml: () -> MlRepository,
    private val io: CoroutineDispatcher = Dispatchers.IO,
    private val nowUs: () -> Long = { System.currentTimeMillis() * 1000L },
    /** The device's UTC offset at a moment, in seconds (DST aware). */
    private val utcOffsetSecs: (nowUs: Long) -> Int = ::systemUtcOffsetSecs,
) {
    private val mutex = Mutex()
    private val _memories = MutableStateFlow<List<Memory>>(emptyList())

    /** Unseen first; empty until [refresh] ran (or when there is nothing to remember today). */
    val memories: StateFlow<List<Memory>> = _memories.asStateFlow()

    /** Re-reads today's memories from core. Errors leave the current list in place. */
    suspend fun refresh() {
        mutex.withLock {
            val fresh = withContext(io) { load() } ?: return
            _memories.value = unseenFirst(fresh)
        }
    }

    /** The memory with [id]: from the current list, else from core (a story restored after process death). */
    suspend fun find(id: String): Memory? {
        _memories.value.firstOrNull { it.id == id }?.let { return it }
        refresh()
        return _memories.value.firstOrNull { it.id == id }
    }

    /** Blocking variant of [find] for callers already on a background thread. */
    fun findBlocking(id: String): Memory? =
        _memories.value.firstOrNull { it.id == id } ?: load()?.firstOrNull { it.id == id }

    /** Marks [id] seen locally (the cover dims, it moves behind the unseen ones) and in core. */
    suspend fun markSeen(id: String) {
        var changed = false
        _memories.update { list ->
            if (list.none { it.id == id && !it.seen }) {
                list
            } else {
                changed = true
                unseenFirst(list.map { if (it.id == id) it.copy(seen = true) else it })
            }
        }
        if (!changed && _memories.value.any { it.id == id }) return
        withContext(io) {
            try {
                ml().markMemorySeen(id)
            } catch (e: CoreError) {
                // Seen state is a local nicety; a failed write only means the cover is bright again tomorrow.
            }
        }
    }

    /** Forgets the list (another account may be signed in next). */
    fun clear() {
        _memories.value = emptyList()
    }

    private fun load(): List<Memory>? {
        val now = nowUs()
        return try {
            ml().memories(now, utcOffsetSecs(now))
        } catch (e: CoreError) {
            null
        }
    }

    companion object {
        /** Unseen first, each group in its original order. */
        fun unseenFirst(list: List<Memory>): List<Memory> =
            if (list.none { it.seen } || list.all { it.seen }) list else list.filter { !it.seen } + list.filter { it.seen }
    }
}

/** The device zone's offset from UTC at [nowUs], in seconds. */
fun systemUtcOffsetSecs(nowUs: Long): Int =
    ZoneId.systemDefault().rules.getOffset(Instant.ofEpochMilli(Math.floorDiv(nowUs, 1000L))).totalSeconds

private object MemoryFeedHolder {
    var graph: AppGraph? = null
    var feed: MemoryFeed? = null
}

/** The process-wide [MemoryFeed] of this [AppGraph] (one per process; AppGraph itself stays untouched). */
fun AppGraph.memoryFeed(): MemoryFeed {
    val graph = this
    synchronized(MemoryFeedHolder) {
        val existing = MemoryFeedHolder.feed
        if (existing != null && MemoryFeedHolder.graph === graph) return existing
        val created = MemoryFeed({ graph.ml })
        MemoryFeedHolder.graph = graph
        MemoryFeedHolder.feed = created
        return created
    }
}
