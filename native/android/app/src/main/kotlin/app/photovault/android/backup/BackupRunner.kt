package app.photovault.android.backup

import androidx.compose.runtime.Immutable
import app.photovault.android.data.Account
import app.photovault.android.data.CoreError
import app.photovault.android.data.CoreRepository
import app.photovault.android.data.UploadStatus
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.coroutineScope
import kotlinx.coroutines.delay
import kotlinx.coroutines.ensureActive
import kotlinx.coroutines.launch
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlin.coroutines.coroutineContext

/** What a backup run is doing right now (the screen and the notification show it). */
@Immutable
sealed interface BackupPhase {
    data object Idle : BackupPhase
    data object Scanning : BackupPhase
    data class Uploading(val status: UploadStatus) : BackupPhase
}

/** Which files still need queuing: never seen, or changed since they were queued (an edit). */
object BackupPlanner {
    fun plan(items: List<LocalMedia>, known: Map<String, LedgerEntry>): List<LocalMedia> =
        items.filter { m -> known[m.key].let { it == null || it.modified != m.modified || it.size != m.size } }
}

/** The ledger's account key: a user id is only unique per server. */
fun accountKey(a: Account): String = "${a.serverUrl.trimEnd('/')}#${a.userId}"

/**
 * One backup run: find new photos and videos in the chosen folders, queue them in photovault-core (each
 * into its folder's album), then run core's upload queue. Everything resumable lives in core: if the run is
 * stopped (Wi-Fi lost, charger pulled, the system's time limit), uploads continue where they stopped on the
 * next run. Runs never overlap ([mutex]).
 */
class BackupRunner(
    private val core: CoreRepository,
    private val albums: BackupCore,
    private val media: MediaSource,
    private val ledger: BackupLedger,
    private val settings: BackupSettingsStore,
    private val pollMs: Long = 1_000L,
    private val clock: () -> Long = System::currentTimeMillis,
) {
    private val mutex = Mutex()

    suspend fun run(onPhase: suspend (BackupPhase) -> Unit = {}): BackupReport = mutex.withLock {
        try {
            runLocked(onPhase)
        } finally {
            onPhase(BackupPhase.Idle)
        }
    }

    private suspend fun runLocked(onPhase: suspend (BackupPhase) -> Unit): BackupReport {
        val s = settings.settings.value
        if (!s.enabled) return report(BackupOutcome.Disabled)
        val account = core.currentAccount() ?: return report(BackupOutcome.NotSignedIn)
        if (media.access() == MediaAccess.None) return report(BackupOutcome.NoPermission)
        val acct = accountKey(account)
        var queued = 0
        var skipped = 0
        var last: UploadStatus? = null
        try {
            // Home or public address for the network the job runs on.
            core.refreshNetwork()
            for (pass in 0 until MAX_PASSES) {
                onPhase(BackupPhase.Scanning)
                val (q, sk) = enqueueNew(acct, s)
                queued += q
                skipped += sk
                val before = core.uploadStatus()
                // Nothing new and nothing waiting: done. (Failed rows are retried once per run, in pass 0.)
                if (q == 0 && before.pending + before.uploading == 0 && (pass > 0 || before.failed == 0)) {
                    last = before
                    break
                }
                val after = upload(onPhase)
                last = after
                // Core stopped early (offline, server unreachable or busy): its queue keeps the rest.
                if (after.pending + after.uploading > 0) {
                    return report(BackupOutcome.Retry, queued, skipped, after.failed)
                }
                ledger.markAllDone(acct)
                // Photos taken during the upload are picked up by one more pass.
                if (q == 0) break
            }
        } catch (e: CancellationException) {
            throw e
        } catch (e: CoreError) {
            val outcome = when (e) {
                is CoreError.Unauthorized, is CoreError.NotLoggedIn -> BackupOutcome.NotSignedIn
                is CoreError.Network, is CoreError.Unreachable, is CoreError.Cancelled -> BackupOutcome.Retry
                is CoreError.Server -> if (e.status == 429 || e.status >= 500) BackupOutcome.Retry else BackupOutcome.Error
                else -> BackupOutcome.Error
            }
            return report(outcome, queued, skipped, last?.failed ?: 0, e.message)
        }
        return report(BackupOutcome.Done, queued, skipped, last?.failed ?: 0)
    }

    /** Queues the new files; returns (queued, skipped). */
    private suspend fun enqueueNew(acct: String, s: BackupSettings): Pair<Int, Int> {
        val status = core.uploadStatus()
        if (status.pending + status.uploading + status.failed == 0) ledger.forgetQueued(acct)
        val folders = s.folders ?: media.folders().filter { it.isCamera }.mapTo(HashSet()) { it.id }
        if (folders.isEmpty()) return 0 to 0
        val todo = BackupPlanner.plan(media.media(folders, s.includeVideos), ledger.entries(acct))
        var queued = 0
        var skipped = 0
        val albumIds = HashMap<String, Long?>()
        for (m in todo) {
            coroutineContext.ensureActive()
            val path = m.path
            if (path == null || !media.readable(path)) {
                skipped++
                continue
            }
            val album = if (albumIds.containsKey(m.folderName)) albumIds[m.folderName]
            else (if (m.folderName.isBlank()) null else albums.backupAlbum(m.folderName)).also { albumIds[m.folderName] = it }
            try {
                core.enqueueUpload(path, album)
            } catch (e: CoreError.Storage) {
                // vanished or unreadable between the scan and now
                skipped++
                continue
            }
            ledger.markQueued(acct, m.key, m.modified, m.size)
            queued++
        }
        return queued to skipped
    }

    /** Runs core's queue, reporting its status every [pollMs] meanwhile. */
    private suspend fun upload(onPhase: suspend (BackupPhase) -> Unit): UploadStatus = coroutineScope {
        // Reported before the queue starts, so the notification (and foreground service) come up at once.
        onPhase(BackupPhase.Uploading(core.uploadStatus()))
        val poll = launch {
            while (true) {
                delay(pollMs)
                onPhase(BackupPhase.Uploading(core.uploadStatus()))
            }
        }
        try {
            core.runUploads()
        } finally {
            poll.cancel()
        }
    }

    private fun report(outcome: BackupOutcome, queued: Int = 0, skipped: Int = 0, failed: Int = 0, message: String? = null) =
        BackupReport(outcome, queued, skipped, failed, message, clock())

    private companion object {
        const val MAX_PASSES = 3
    }
}
