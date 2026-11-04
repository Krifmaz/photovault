package app.photovault.android.backup

import app.photovault.android.data.CoreError
import app.photovault.android.data.CoreRepository
import app.photovault.android.data.UploadStatus
import app.photovault.android.data.fake.FakeCore
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.MutableStateFlow

/** The demo core with a queue that behaves like core's: enqueue, run (or stop with [stopWith]), status. */
class QueueCore(val base: FakeCore = FakeCore(latencyMs = 0)) : CoreRepository by base {
    val enqueued = ArrayList<Pair<String, Long?>>()
    var pending = 0
    var failed = 0
    var stopWith: CoreError? = null
    /** Network stop: core returns Ok with the rest still pending. */
    var stopKeepsPending = false
    var wipeOnRun = false
    var runs = 0
    var unreadable = HashSet<String>()

    override fun enqueueUpload(path: String, collectionId: Long?) {
        if (path in unreadable) throw CoreError.Storage("$path: gone")
        enqueued += path to collectionId
        pending++
    }

    override suspend fun runUploads(): UploadStatus {
        runs++
        stopWith?.let { throw it }
        if (!stopKeepsPending) pending = 0
        return uploadStatus()
    }

    override fun uploadStatus(): UploadStatus = UploadStatus(pending, 0, failed, 0, pending * 100L)

    /** Sign-out: core wipes its queue. */
    fun wipeQueue() {
        pending = 0
        failed = 0
    }
}

class FakeMedia(
    var access: MediaAccess = MediaAccess.Full,
    var folders: List<LocalFolder> = listOf(
        LocalFolder("cam", "Camera", 2, isCamera = true),
        LocalFolder("shots", "Screenshots", 1, isCamera = false),
    ),
) : MediaSource {
    val items = ArrayList<LocalMedia>()
    val unreadable = HashSet<String>()

    fun add(id: Int, folder: String = "cam", name: String = "Camera", modified: Long = 100L + id, video: Boolean = false) {
        items += LocalMedia("external:${if (video) "v" else "i"}:$id", "/sdcard/DCIM/$name/$id.jpg", folder, name, modified, 1000L, video)
    }

    override fun access(): MediaAccess = access
    override fun folders(): List<LocalFolder> = if (access == MediaAccess.None) emptyList() else folders
    override fun media(folderIds: Set<String>, includeVideos: Boolean): List<LocalMedia> =
        items.filter { it.folderId in folderIds && (includeVideos || !it.isVideo) }.sortedByDescending { it.modified }
    override fun readable(path: String): Boolean = path !in unreadable
}

class RecordingScheduler : BackupScheduler {
    val applied = ArrayList<Pair<BackupSettings, Boolean>>()
    var nowRuns = 0
    val state = MutableStateFlow(RunState.Off)
    override fun apply(settings: BackupSettings, changed: Boolean) { applied += settings to changed }
    override fun runNow(settings: BackupSettings) { nowRuns++ }
    override val runState: Flow<RunState> = state
}
