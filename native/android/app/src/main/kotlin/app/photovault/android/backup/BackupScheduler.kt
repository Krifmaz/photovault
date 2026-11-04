package app.photovault.android.backup

import android.content.Context
import android.provider.MediaStore
import androidx.work.BackoffPolicy
import androidx.work.Constraints
import androidx.work.CoroutineWorker
import androidx.work.ExistingPeriodicWorkPolicy
import androidx.work.ExistingWorkPolicy
import androidx.work.NetworkType
import androidx.work.OneTimeWorkRequestBuilder
import androidx.work.OutOfQuotaPolicy
import androidx.work.PeriodicWorkRequestBuilder
import androidx.work.WorkInfo
import androidx.work.WorkManager
import androidx.work.WorkerParameters
import androidx.work.workDataOf
import app.photovault.android.appGraph
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.flow.distinctUntilChanged
import java.util.concurrent.TimeUnit

/** Whether a backup run is scheduled and what it waits for. */
enum class RunState { Off, Waiting, Running }

interface BackupScheduler {
    /** Brings the scheduled work in line with [settings]; [changed] restarts a waiting run with new constraints. */
    fun apply(settings: BackupSettings, changed: Boolean)
    /** "Back up now": runs as soon as the network allows (the charging option is not waited for). */
    fun runNow(settings: BackupSettings)
    val runState: Flow<RunState>
}

/**
 * WorkManager plan:
 * - `backup-run` (unique, one-time): the [BackupWorker] with the user's constraints (Wi-Fi or any network,
 *   charging, battery not low). Started by the kicks below; KEEP, so a waiting or running run is never doubled.
 * - `backup-periodic`: a kick every hour, so backup also happens when no new photo arrives (retries, new folder).
 * - `backup-media`: a kick when MediaStore changes (a new photo), re-armed by itself after each change.
 * - `backup-now`: the user's "Back up now" (expedited when quota allows).
 */
class WorkBackupScheduler(private val context: Context) : BackupScheduler {
    private val wm: WorkManager get() = WorkManager.getInstance(context)

    override fun apply(settings: BackupSettings, changed: Boolean) {
        if (!settings.enabled) {
            listOf(RUN, NOW, PERIODIC, MEDIA).forEach { wm.cancelUniqueWork(it) }
            return
        }
        wm.enqueueUniquePeriodicWork(
            PERIODIC,
            ExistingPeriodicWorkPolicy.KEEP,
            PeriodicWorkRequestBuilder<BackupKickWorker>(1, TimeUnit.HOURS).build(),
        )
        armMediaTrigger(context, ExistingWorkPolicy.KEEP)
        enqueueRun(context, settings, if (changed) ExistingWorkPolicy.REPLACE else ExistingWorkPolicy.KEEP)
    }

    override fun runNow(settings: BackupSettings) {
        if (!settings.enabled) return
        val req = OneTimeWorkRequestBuilder<BackupWorker>()
            .setConstraints(Constraints.Builder().setRequiredNetworkType(network(settings)).build())
            .setExpedited(OutOfQuotaPolicy.RUN_AS_NON_EXPEDITED_WORK_REQUEST)
            .build()
        wm.enqueueUniqueWork(NOW, ExistingWorkPolicy.KEEP, req)
    }

    override val runState: Flow<RunState> =
        combine(wm.getWorkInfosForUniqueWorkFlow(RUN), wm.getWorkInfosForUniqueWorkFlow(NOW)) { a, b ->
            val states = (a + b).map { it.state }
            when {
                WorkInfo.State.RUNNING in states -> RunState.Running
                states.any { it == WorkInfo.State.ENQUEUED || it == WorkInfo.State.BLOCKED } -> RunState.Waiting
                else -> RunState.Off
            }
        }.distinctUntilChanged()

    companion object {
        const val RUN = "backup-run"
        const val NOW = "backup-now"
        const val PERIODIC = "backup-periodic"
        const val MEDIA = "backup-media"
        const val KEY_REARM = "rearm"

        fun network(s: BackupSettings) = if (s.wifiOnly) NetworkType.UNMETERED else NetworkType.CONNECTED

        fun constraints(s: BackupSettings): Constraints = Constraints.Builder()
            .setRequiredNetworkType(network(s))
            .setRequiresCharging(s.chargingOnly)
            // While charging the battery level does not matter.
            .setRequiresBatteryNotLow(!s.chargingOnly)
            .build()

        fun enqueueRun(context: Context, s: BackupSettings, policy: ExistingWorkPolicy) {
            val req = OneTimeWorkRequestBuilder<BackupWorker>()
                .setConstraints(constraints(s))
                .setBackoffCriteria(BackoffPolicy.EXPONENTIAL, 1, TimeUnit.MINUTES)
                .build()
            WorkManager.getInstance(context).enqueueUniqueWork(RUN, policy, req)
        }

        /**
         * A kick that waits for a MediaStore change. Called with APPEND_OR_REPLACE from the kick itself, the new
         * kick is chained after the running one and starts listening once that has finished.
         */
        fun armMediaTrigger(context: Context, policy: ExistingWorkPolicy) {
            val c = Constraints.Builder()
                .addContentUriTrigger(MediaStore.Images.Media.EXTERNAL_CONTENT_URI, true)
                .addContentUriTrigger(MediaStore.Video.Media.EXTERNAL_CONTENT_URI, true)
                // A burst of shots becomes one kick.
                .setTriggerContentUpdateDelay(10, TimeUnit.SECONDS)
                .setTriggerContentMaxDelay(2, TimeUnit.MINUTES)
                .build()
            val req = OneTimeWorkRequestBuilder<BackupKickWorker>()
                .setConstraints(c)
                .setInputData(workDataOf(KEY_REARM to true))
                .build()
            WorkManager.getInstance(context).enqueueUniqueWork(MEDIA, policy, req)
        }
    }
}

/** Starts `backup-run` (unless one is already waiting or running); a media kick re-arms itself. */
class BackupKickWorker(context: Context, params: WorkerParameters) : CoroutineWorker(context, params) {
    override suspend fun doWork(): Result {
        val settings = applicationContext.appGraph.backup.settings.settings.value
        if (!settings.enabled) return Result.success()
        WorkBackupScheduler.enqueueRun(applicationContext, settings, ExistingWorkPolicy.KEEP)
        if (inputData.getBoolean(WorkBackupScheduler.KEY_REARM, false)) {
            WorkBackupScheduler.armMediaTrigger(applicationContext, ExistingWorkPolicy.APPEND_OR_REPLACE)
        }
        return Result.success()
    }
}
