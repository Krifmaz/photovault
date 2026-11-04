package app.photovault.android.ui.viewer

import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import app.photovault.android.data.CoreError
import app.photovault.android.data.CoreRepository
import app.photovault.android.data.FileKind
import app.photovault.android.data.TimelineItem
import app.photovault.android.data.TimelineStore
import app.photovault.android.data.VideoRepository
import app.photovault.android.ui.viewer.video.PlaybackEngine
import app.photovault.android.ui.viewer.video.VideoPlaybackController
import kotlinx.coroutines.CoroutineDispatcher
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
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
    /** Outlives this view model: deletes the decrypted originals after the viewer closes, off the main thread. */
    private val cleanupScope: CoroutineScope = CoroutineScope(SupervisorJob() + io),
    /** The timeline position the viewer opens at (loaded before the pager appears). */
    startIndex: Int = 0,
    /** Core's streamable videos (contract v1.2); without it, v1's `streamPlaylist` for items with `hasStream`. */
    private val videos: VideoRepository? = null,
    /** Creates the viewer's one player on the first video (an ExoPlayer in the app, a fake in tests). */
    engineFactory: () -> PlaybackEngine = { error("no playback engine") },
) : ViewModel() {

    val items: StateFlow<List<TimelineItem>> = store.items

    /** core's timeline count at the last refresh. */
    val total: StateFlow<Long> = store.total

    /** False until the timeline (and the page the viewer opens at) is loaded. */
    val loaded: StateFlow<Boolean> = store.loaded
    private val cacheLock = Mutex()

    /** The one player of all video pages; released (and core's streams ended) when the viewer closes. */
    val playback = VideoPlaybackController(
        scope = viewModelScope,
        engineFactory = engineFactory,
        resolve = ::video,
        onReleased = { videos?.stopStreams() },
    )

    init {
        videos?.viewerOpened()
        // After process death the viewer is restored on its own: the shared store starts empty and the
        // timeline screen (which normally fills it) is not composed. Load it here, up to the opened photo.
        viewModelScope.launch {
            if (!store.loaded.value) store.refresh()
            store.ensureLoaded(startIndex + 1)
        }
    }

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

    /**
     * A video's source: core's stream of Ente's HLS preview when there is one, else the downloaded original
     * (the same player plays both). Core knows the account's own streams locally and asks the server for
     * videos shared by others, so it is asked for every video; a failing stream falls back to the original.
     */
    suspend fun video(item: TimelineItem): VideoSource {
        require(item.kind == FileKind.Video || item.kind == FileKind.LivePhoto)
        val stream = try {
            when {
                videos != null -> videos.videoStream(item.id)?.playlistUrl
                item.hasStream -> core.streamPlaylist(item.id)
                else -> null
            }
        } catch (e: CoreError) {
            if (e is CoreError.NotLoggedIn || e is CoreError.Unauthorized) throw e
            null
        }
        if (stream != null) return VideoSource(stream, isHls = true)
        return VideoSource("file://" + original(item).absolutePath, isHls = false)
    }

    private fun trim(keep: File) {
        val others = cacheDir.listFiles { f -> f.name.endsWith(".bin") && f != keep } ?: return
        if (others.size < KEEP_FILES) return
        others.sortedByDescending { it.lastModified() }.drop(KEEP_FILES - 1).forEach { it.delete() }
    }

    override fun onCleared() {
        playback.release()
        // decrypted originals do not outlive the viewer; deleting them is file I/O, so not on the main thread
        val dir = cacheDir
        cleanupScope.launch { dir.listFiles()?.forEach { it.delete() } }
    }

    companion object {
        const val KEEP_FILES = 6
    }
}
