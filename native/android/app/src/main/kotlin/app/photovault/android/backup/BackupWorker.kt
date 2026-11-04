package app.photovault.android.backup

import android.content.Context
import android.util.Log
import androidx.work.CoroutineWorker
import androidx.work.ForegroundInfo
import androidx.work.WorkerParameters
import app.photovault.android.appGraph
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext

/**
 * One backup run ([BackupRunner]) as WorkManager work. While uploading it asks to run as a foreground
 * service (dataSync) with a progress notification, so a long upload is not cut off after the usual job time.
 * Android may refuse that from the background (Android 12+); the run then continues as a normal job, and
 * whatever it does not finish is resumed by the next run from core's persisted queue.
 */
class BackupWorker(context: Context, params: WorkerParameters) : CoroutineWorker(context, params) {

    override suspend fun doWork(): Result {
        val backup = applicationContext.appGraph.backup
        var foreground = false
        var triedForeground = false
        val report = withContext(Dispatchers.IO) {
            backup.runner().run { phase ->
                backup.publish(phase)
                if (phase is BackupPhase.Uploading) {
                    if (!triedForeground) {
                        triedForeground = true
                        foreground = tryForeground(backup.notifications.foregroundInfo(phase.status))
                    } else if (foreground) {
                        tryForeground(backup.notifications.foregroundInfo(phase.status))
                    }
                }
            }
        }
        backup.settings.record(report)
        return when (report.outcome) {
            BackupOutcome.Retry -> if (runAttemptCount < MAX_RETRIES) Result.retry() else Result.success()
            BackupOutcome.Error -> Result.failure()
            else -> Result.success()
        }
    }

    override suspend fun getForegroundInfo(): ForegroundInfo =
        applicationContext.appGraph.backup.notifications.foregroundInfo(null)

    private suspend fun tryForeground(info: ForegroundInfo): Boolean = try {
        setForeground(info)
        true
    } catch (e: IllegalStateException) {
        // ForegroundServiceStartNotAllowedException (an IllegalStateException) from the background on Android 12+
        Log.i(TAG, "backup continues without a foreground service: ${e.message}")
        false
    }

    private companion object {
        const val TAG = "PhotovaultBackup"
        // The periodic kick starts a fresh run after this.
        const val MAX_RETRIES = 6
    }
}
