package app.photovault.android.backup

import android.content.Context
import app.photovault.android.data.CoreRepository
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow

/**
 * Camera backup's long-lived objects (AppGraph.backup). Everything here touches disk on first use, so it is
 * first used from background threads: the app's start-up check, the worker, the backup screen's IO dispatcher.
 */
class BackupGraph(private val context: Context, private val core: () -> CoreRepository) {
    val settings: BackupSettingsStore by lazy { PrefsBackupSettingsStore(context) }
    val media: MediaSource by lazy { MediaStoreSource(context) }
    val ledger: BackupLedger by lazy { SqliteBackupLedger(context) }
    val scheduler: BackupScheduler by lazy { WorkBackupScheduler(context) }
    val notifications: BackupNotifications by lazy { BackupNotifications(context) }

    private val runner: BackupRunner by lazy {
        val c = core()
        BackupRunner(c, c.backupCore(), media, ledger, settings)
    }

    fun runner(): BackupRunner = runner

    private val _phase = MutableStateFlow<BackupPhase>(BackupPhase.Idle)

    /** The running backup's phase in this process (Idle when none). */
    val phase: StateFlow<BackupPhase> = _phase.asStateFlow()

    fun publish(p: BackupPhase) {
        _phase.value = p
    }

    /** At process start: make sure the scheduled work matches the settings (e.g. after an app update). */
    fun start() {
        scheduler.apply(settings.settings.value, changed = false)
    }
}
