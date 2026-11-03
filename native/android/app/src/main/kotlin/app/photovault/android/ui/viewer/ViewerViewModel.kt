package app.photovault.android.ui.viewer

import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import app.photovault.android.data.CoreRepository
import app.photovault.android.data.FileKind
import app.photovault.android.data.TimelineItem
import app.photovault.android.data.TimelineStore
import kotlinx.coroutines.CoroutineDispatcher
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.launch
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withContext
import java.io.File

/** What a video page plays: an HLS playlist (Ente streamable) or a downloaded file. */
data class VideoSource(val uri: String, val isHls: Boolean)

/**
 * The full-screen viewer over the same [TimelineStore] the grid shows. Originals are decrypted by core's
 * `download` into the app-private [cacheDir] and only the last [KEEP_FILES] are kept.
 */
class ViewerViewModel(
    private val core: CoreRepository,
    private val store: TimelineStore,
    private val cacheDir: File,
    private val io: CoroutineDispatcher = Dispatchers.IO,
) : ViewModel() {

    val items: StateFlow<List<TimelineItem>> = store.items
    private val cacheLock = Mutex()

    /** The pager settled on [index]: keep loading ahead of it. */
    fun onPage(index: Int) {
        val want = index + TimelineStore.AHEAD
        if (want <= store.items.value.size) return
        viewModelScope.launch { store.ensureLoaded(want) }
    }

    /** The decrypted original for a photo, downloaded on first view. */
    suspend fun original(item: TimelineItem): File = withContext(io) {
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

    /** A video's stream: Ente's HLS playlist when the file has one, else the downloaded file. */
    suspend fun video(item: TimelineItem): VideoSource {
        require(item.kind == FileKind.Video || item.kind == FileKind.LivePhoto)
        if (item.hasStream) {
            val playlist = core.streamPlaylist(item.id)
            if (playlist != null) return VideoSource(playlist, isHls = true)
        }
        return VideoSource("file://" + original(item).absolutePath, isHls = false)
    }

    private fun trim(keep: File) {
        val others = cacheDir.listFiles { f -> f.name.endsWith(".bin") && f != keep } ?: return
        if (others.size < KEEP_FILES) return
        others.sortedByDescending { it.lastModified() }.drop(KEEP_FILES - 1).forEach { it.delete() }
    }

    override fun onCleared() {
        // decrypted originals do not outlive the viewer
        cacheDir.listFiles()?.forEach { it.delete() }
    }

    companion object {
        const val KEEP_FILES = 6
    }
}
