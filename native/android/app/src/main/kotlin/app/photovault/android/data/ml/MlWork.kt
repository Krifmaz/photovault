package app.photovault.android.data.ml

import android.content.Context
import android.content.SharedPreferences
import android.os.Process
import androidx.work.BackoffPolicy
import androidx.work.Constraints
import androidx.work.CoroutineWorker
import androidx.work.ExistingWorkPolicy
import androidx.work.NetworkType
import androidx.work.OneTimeWorkRequestBuilder
import androidx.work.WorkManager
import androidx.work.WorkerParameters
import androidx.work.workDataOf
import app.photovault.android.appGraph
import app.photovault.android.data.CoreError
import kotlinx.coroutines.CoroutineDispatcher
import kotlinx.coroutines.asCoroutineDispatcher
import kotlinx.coroutines.withContext
import java.util.concurrent.Executors
import java.util.concurrent.TimeUnit

/*
 * Background ML (faces + magic search) with WorkManager, like Ente's ML service runs its indexing only when the
 * phone can spare it (Ente: mobile/apps/photos/lib/services/machine_learning/ml_service.dart, which also waits
 * for charging on mobile by default).
 *
 * - After each library sync the app calls [MlScheduler.afterSync]: [MlSyncWorker] runs core's `sync_ml` (pull the
 *   mldata and people other devices computed; cheap when nothing changed) on any network.
 * - When files are left to index and this build can run the models, [MlIndexWorker] runs `run_ml_indexing` in
 *   batches of [MlWork.BATCH] while the constraints hold: charging (the "only while charging" setting, on by
 *   default), battery not low, and an unmetered network while the models still have to be downloaded from the
 *   user's server. WorkManager stops the worker (cancelling the coroutine, which cancels core's call cleanly)
 *   as soon as a constraint fails and runs it again when they hold.
 * - All core calls run on one background-priority thread, so the UI keeps its full refresh rate.
 *
 * Based on Ente (ente.com).
 */

/** Schedules and cancels the background ML work. */
interface MlScheduler {
    /** The app finished a library sync: pull ML data (and index if needed) when ML is on. */
    fun afterSync()

    /** ML was switched on, or a setting changed: (re)schedule with the current settings. */
    fun reschedule()

    /** ML was switched off: stop and drop all pending ML work. */
    fun cancel()
}

/** The "index only while charging" setting, behind an interface so tests need no Android preferences. */
interface ChargingSetting {
    /** May read a file: call off the main thread. */
    fun get(): Boolean
    fun set(value: Boolean)
}

/** The ML settings kept on this device (not secret: a switch, no data derived from photos). */
class MlPrefs(context: Context) : ChargingSetting {
    private val app = context.applicationContext
    private val prefs: SharedPreferences by lazy { app.getSharedPreferences(FILE, Context.MODE_PRIVATE) }

    /** Index only while charging (default on). Reads the preferences file: call off the main thread. */
    var indexOnlyWhileCharging: Boolean
        get() = prefs.getBoolean(KEY_CHARGING, true)
        set(value) {
            prefs.edit().putBoolean(KEY_CHARGING, value).apply()
        }

    override fun get(): Boolean = indexOnlyWhileCharging

    override fun set(value: Boolean) {
        indexOnlyWhileCharging = value
    }

    private companion object {
        const val FILE = "photovault_ml"
        const val KEY_CHARGING = "index_only_while_charging"
    }
}

/** What the indexing work waits for, as plain values (mapped to WorkManager's [Constraints]). */
data class IndexConstraints(val requiresCharging: Boolean, val batteryNotLow: Boolean, val unmetered: Boolean)

object MlWork {
    const val SYNC_WORK = "photovault-ml-sync"
    const val INDEX_WORK = "photovault-ml-index"

    /** Files per `run_ml_indexing` call: small enough to stop quickly, large enough to amortise the call. */
    const val BATCH = 25

    /** WorkManager gives a worker about 10 minutes; leave in time and continue in a follow-up run. */
    const val RUN_BUDGET_MS = 8L * 60 * 1000

    /** The indexing constraints for the current setting and model state. */
    fun indexConstraints(onlyWhileCharging: Boolean, modelsReady: Boolean): IndexConstraints =
        IndexConstraints(requiresCharging = onlyWhileCharging, batteryNotLow = true, unmetered = !modelsReady)

    /** Whether to queue indexing after a sync. */
    fun shouldIndex(status: MlStatus): Boolean =
        status.enabled && status.inferenceAvailable && status.filesPending > 0

    fun scheduler(context: Context): MlScheduler = WorkManagerMlScheduler(context.applicationContext)

    internal fun enqueueIndex(context: Context, status: MlStatus, policy: ExistingWorkPolicy) {
        val c = indexConstraints(MlPrefs(context).indexOnlyWhileCharging, status.modelsReady)
        val constraints = Constraints.Builder()
            .setRequiresCharging(c.requiresCharging)
            .setRequiresBatteryNotLow(c.batteryNotLow)
            .setRequiredNetworkType(if (c.unmetered) NetworkType.UNMETERED else NetworkType.CONNECTED)
            .build()
        val request = OneTimeWorkRequestBuilder<MlIndexWorker>()
            .setConstraints(constraints)
            .setBackoffCriteria(BackoffPolicy.EXPONENTIAL, 5, TimeUnit.MINUTES)
            .build()
        WorkManager.getInstance(context).enqueueUniqueWork(INDEX_WORK, policy, request)
    }

    /** [replaceIndex]: give queued or running indexing the current settings (else a running batch is kept). */
    internal fun enqueueSync(context: Context, replaceIndex: Boolean) {
        val request = OneTimeWorkRequestBuilder<MlSyncWorker>()
            .setInputData(workDataOf(KEY_REPLACE_INDEX to replaceIndex))
            .setConstraints(Constraints.Builder().setRequiredNetworkType(NetworkType.CONNECTED).build())
            .setBackoffCriteria(BackoffPolicy.EXPONENTIAL, 1, TimeUnit.MINUTES)
            .build()
        WorkManager.getInstance(context).enqueueUniqueWork(SYNC_WORK, ExistingWorkPolicy.REPLACE, request)
    }

    /**
     * One background-priority thread for every ML call from the app. Core runs the heavy parts on its own
     * blocking pool; this keeps the calling side (polling core's futures, callbacks) off the UI's cores too.
     */
    internal val dispatcher: CoroutineDispatcher by lazy {
        Executors.newSingleThreadExecutor { r ->
            Thread({
                Process.setThreadPriority(Process.THREAD_PRIORITY_BACKGROUND)
                r.run()
            }, "photovault-ml").apply { isDaemon = true }
        }.asCoroutineDispatcher()
    }

    internal const val KEY_REPLACE_INDEX = "replace_index"

    /** Network-ish failures are worth a retry; the rest (signed out, ML off) are not. */
    internal fun retryable(e: CoreError): Boolean =
        e is CoreError.Network || e is CoreError.Unreachable || e is CoreError.Server
}

private class WorkManagerMlScheduler(private val context: Context) : MlScheduler {
    // WorkManager's enqueue/cancel calls only post to its own executor; MlPrefs is read by the sync worker.
    override fun afterSync() = MlWork.enqueueSync(context, replaceIndex = false)

    override fun reschedule() = MlWork.enqueueSync(context, replaceIndex = true)

    override fun cancel() {
        val wm = WorkManager.getInstance(context)
        wm.cancelUniqueWork(MlWork.SYNC_WORK)
        wm.cancelUniqueWork(MlWork.INDEX_WORK)
    }
}

/** Pulls ML data from the server after a library sync, then queues indexing when there is work left. */
class MlSyncWorker(context: Context, params: WorkerParameters) : CoroutineWorker(context, params) {
    override suspend fun doWork(): Result = withContext(MlWork.dispatcher) {
        val ml = applicationContext.appGraph.ml
        try {
            if (!ml.mlStatus().enabled) return@withContext Result.success()
            val status = ml.syncMl(null)
            // KEEP after an ordinary sync, so a batch in progress is not interrupted; REPLACE after a settings
            // change, so the queued run gets the new constraints.
            val policy = if (inputData.getBoolean(MlWork.KEY_REPLACE_INDEX, false)) ExistingWorkPolicy.REPLACE else ExistingWorkPolicy.KEEP
            if (MlWork.shouldIndex(status)) MlWork.enqueueIndex(applicationContext, status, policy)
            Result.success()
        } catch (e: CoreError) {
            if (MlWork.retryable(e) && runAttemptCount < MAX_ATTEMPTS) Result.retry() else Result.success()
        }
    }

    private companion object {
        const val MAX_ATTEMPTS = 5
    }
}

/** Indexes files on this device in batches until done, stopped by WorkManager, or out of time. */
class MlIndexWorker(context: Context, params: WorkerParameters) : CoroutineWorker(context, params) {
    override suspend fun doWork(): Result = withContext(MlWork.dispatcher) {
        val ml = applicationContext.appGraph.ml
        val started = System.currentTimeMillis()
        try {
            var status = ml.mlStatus()
            while (!isStopped) {
                if (!status.enabled || !status.inferenceAvailable) return@withContext Result.success()
                val before = status.filesIndexed
                status = ml.runMlIndexing(MlWork.BATCH, null)
                if (status.filesPending == 0L) return@withContext Result.success()
                if (status.filesIndexed == before) {
                    // No progress (a model or server problem, see lastError): back off instead of spinning.
                    return@withContext if (runAttemptCount < MAX_ATTEMPTS) Result.retry() else Result.success()
                }
                if (System.currentTimeMillis() - started > MlWork.RUN_BUDGET_MS) {
                    // Continue in a fresh run once this one has finished (and only while the constraints hold).
                    MlWork.enqueueIndex(applicationContext, status, ExistingWorkPolicy.APPEND_OR_REPLACE)
                    return@withContext Result.success()
                }
            }
            Result.success()
        } catch (e: CoreError) {
            if (MlWork.retryable(e) && runAttemptCount < MAX_ATTEMPTS) Result.retry() else Result.success()
        }
    }

    private companion object {
        const val MAX_ATTEMPTS = 5
    }
}
