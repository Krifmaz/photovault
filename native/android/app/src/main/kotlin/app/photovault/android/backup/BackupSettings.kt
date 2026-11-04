package app.photovault.android.backup

import android.content.Context
import android.content.SharedPreferences
import androidx.compose.runtime.Immutable
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow

/** The user's backup choices. Defaults follow Ente's app: off until turned on, Wi-Fi only, videos included. */
@Immutable
data class BackupSettings(
    val enabled: Boolean = false,
    /** Chosen folder (MediaStore bucket) ids; null until the user picks, meaning the camera folders. */
    val folders: Set<String>? = null,
    val wifiOnly: Boolean = true,
    val chargingOnly: Boolean = false,
    val includeVideos: Boolean = true,
)

/** How the last backup run ended. */
enum class BackupOutcome { Done, Disabled, NotSignedIn, NoPermission, Retry, Error }

/** The last run, shown on the backup screen. */
@Immutable
data class BackupReport(
    val outcome: BackupOutcome,
    /** Photos and videos newly queued in this run. */
    val queued: Int = 0,
    /** Files that could not be read (moved, deleted, unreadable path). */
    val skipped: Int = 0,
    /** Uploads that failed and are left in core's queue. */
    val failed: Int = 0,
    val message: String? = null,
    val atMs: Long = 0L,
)

interface BackupSettingsStore {
    val settings: StateFlow<BackupSettings>
    val lastReport: StateFlow<BackupReport?>
    fun update(change: (BackupSettings) -> BackupSettings): BackupSettings
    fun record(report: BackupReport)
}

/** Kept in SharedPreferences. The constructor reads the file: create it off the main thread. */
class PrefsBackupSettingsStore(context: Context) : BackupSettingsStore {
    private val prefs: SharedPreferences = context.getSharedPreferences("backup", Context.MODE_PRIVATE)
    private val _settings = MutableStateFlow(read())
    private val _last = MutableStateFlow(readReport())

    override val settings: StateFlow<BackupSettings> = _settings.asStateFlow()
    override val lastReport: StateFlow<BackupReport?> = _last.asStateFlow()

    override fun update(change: (BackupSettings) -> BackupSettings): BackupSettings = synchronized(this) {
        val next = change(_settings.value)
        val edit = prefs.edit()
            .putBoolean(K_ENABLED, next.enabled)
            .putBoolean(K_WIFI, next.wifiOnly)
            .putBoolean(K_CHARGING, next.chargingOnly)
            .putBoolean(K_VIDEOS, next.includeVideos)
        if (next.folders == null) edit.remove(K_FOLDERS) else edit.putStringSet(K_FOLDERS, HashSet(next.folders))
        edit.apply()
        _settings.value = next
        next
    }

    override fun record(report: BackupReport) {
        prefs.edit()
            .putString(K_LAST_OUTCOME, report.outcome.name)
            .putInt(K_LAST_QUEUED, report.queued)
            .putInt(K_LAST_SKIPPED, report.skipped)
            .putInt(K_LAST_FAILED, report.failed)
            .putString(K_LAST_MESSAGE, report.message)
            .putLong(K_LAST_AT, report.atMs)
            .apply()
        _last.value = report
    }

    private fun read() = BackupSettings(
        enabled = prefs.getBoolean(K_ENABLED, false),
        folders = prefs.getStringSet(K_FOLDERS, null)?.toSet(),
        wifiOnly = prefs.getBoolean(K_WIFI, true),
        chargingOnly = prefs.getBoolean(K_CHARGING, false),
        includeVideos = prefs.getBoolean(K_VIDEOS, true),
    )

    private fun readReport(): BackupReport? {
        val outcome = prefs.getString(K_LAST_OUTCOME, null)?.let { n -> BackupOutcome.entries.firstOrNull { it.name == n } }
            ?: return null
        return BackupReport(
            outcome = outcome,
            queued = prefs.getInt(K_LAST_QUEUED, 0),
            skipped = prefs.getInt(K_LAST_SKIPPED, 0),
            failed = prefs.getInt(K_LAST_FAILED, 0),
            message = prefs.getString(K_LAST_MESSAGE, null),
            atMs = prefs.getLong(K_LAST_AT, 0L),
        )
    }

    private companion object {
        const val K_ENABLED = "enabled"
        const val K_FOLDERS = "folders"
        const val K_WIFI = "wifi_only"
        const val K_CHARGING = "charging_only"
        const val K_VIDEOS = "include_videos"
        const val K_LAST_OUTCOME = "last_outcome"
        const val K_LAST_QUEUED = "last_queued"
        const val K_LAST_SKIPPED = "last_skipped"
        const val K_LAST_FAILED = "last_failed"
        const val K_LAST_MESSAGE = "last_message"
        const val K_LAST_AT = "last_at"
    }
}

/** In memory, for tests. */
class MemoryBackupSettingsStore(initial: BackupSettings = BackupSettings()) : BackupSettingsStore {
    private val _settings = MutableStateFlow(initial)
    private val _last = MutableStateFlow<BackupReport?>(null)
    override val settings: StateFlow<BackupSettings> = _settings.asStateFlow()
    override val lastReport: StateFlow<BackupReport?> = _last.asStateFlow()
    override fun update(change: (BackupSettings) -> BackupSettings): BackupSettings =
        synchronized(this) { change(_settings.value).also { _settings.value = it } }
    override fun record(report: BackupReport) { _last.value = report }
}
