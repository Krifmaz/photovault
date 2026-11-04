package app.photovault.android.ui.memories

import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import app.photovault.android.data.CoreRepository
import app.photovault.android.data.FileKind
import app.photovault.android.data.TimelineItem
import app.photovault.android.data.ml.Memory
import kotlinx.coroutines.CoroutineDispatcher
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withContext
import java.io.File

/** What the story screen shows. [memory] is null while loading, or when the memory is gone ([missing]). */
data class StoryState(val memory: Memory? = null, val index: Int = 0, val missing: Boolean = false)

/**
 * One memory as a full-screen story: a page per photo, advanced by the screen's timer or taps. Opening it
 * marks the memory seen (core's `mark_memory_seen`). Images show their thumbnail at once and the decrypted
 * original once downloaded into the app-private [cacheDir]; at most [KEEP_FILES] are kept and all are
 * deleted when the story closes.
 *
 * Based on Ente (ente.com): mobile/apps/photos/lib/ui/home/memories/full_screen_memory.dart.
 */
class MemoryStoryViewModel(
    private val feed: MemoryFeed,
    private val core: CoreRepository,
    private val cacheDir: File,
    private val memoryId: String,
    startIndex: Int = 0,
    private val io: CoroutineDispatcher = Dispatchers.IO,
    /** Outlives this view model: deletes the decrypted originals after the story closes. */
    private val cleanupScope: CoroutineScope = CoroutineScope(SupervisorJob() + io),
) : ViewModel() {
    private val _state = MutableStateFlow(StoryState(index = startIndex.coerceAtLeast(0)))
    val state: StateFlow<StoryState> = _state.asStateFlow()
    private val cacheLock = Mutex()

    init {
        viewModelScope.launch {
            val memory = feed.find(memoryId)
            if (memory == null || memory.items.isEmpty()) {
                _state.value = StoryState(missing = true)
                return@launch
            }
            _state.value = _state.value.copy(memory = memory, index = _state.value.index.coerceIn(0, memory.items.lastIndex))
            feed.markSeen(memoryId)
        }
    }

    val current: TimelineItem? get() = _state.value.let { s -> s.memory?.items?.getOrNull(s.index) }

    /** Next photo; false at the last one (the screen then closes). */
    fun next(): Boolean {
        val s = _state.value
        val items = s.memory?.items ?: return false
        if (s.index >= items.lastIndex) return false
        _state.value = s.copy(index = s.index + 1)
        return true
    }

    /** Previous photo (stays on the first). */
    fun previous() {
        val s = _state.value
        if (s.memory == null || s.index == 0) return
        _state.value = s.copy(index = s.index - 1)
    }

    /** Back from the photo viewer, which may have been paged to another photo of this memory. */
    fun jumpTo(index: Int) {
        val s = _state.value
        val items = s.memory?.items ?: return
        _state.value = s.copy(index = index.coerceIn(0, items.lastIndex))
    }

    /** The decrypted original of an image (null for videos: they show their thumbnail and play in the viewer). */
    suspend fun original(item: TimelineItem): File? {
        if (item.kind == FileKind.Video) return null
        return withContext(io) {
            val dest = File(cacheDir, "${item.id}.bin")
            if (!dest.isFile || dest.length() == 0L) {
                cacheDir.mkdirs()
                val tmp = File.createTempFile("${item.id}-", ".part", cacheDir)
                try {
                    val written = File(core.download(item.id, tmp.path))
                    if (!written.renameTo(dest)) {
                        written.copyTo(dest, overwrite = true)
                        written.delete()
                    }
                } finally {
                    tmp.delete()
                }
            } else {
                dest.setLastModified(System.currentTimeMillis())
            }
            cacheLock.withLock { trim(keep = dest) }
            dest
        }
    }

    private fun trim(keep: File) {
        val others = cacheDir.listFiles { f -> f.name.endsWith(".bin") && f != keep } ?: return
        if (others.size < KEEP_FILES) return
        others.sortedByDescending { it.lastModified() }.drop(KEEP_FILES - 1).forEach { it.delete() }
    }

    override fun onCleared() {
        val dir = cacheDir
        cleanupScope.launch { dir.listFiles()?.forEach { it.delete() } }
    }

    companion object {
        const val KEEP_FILES = 3
    }
}
