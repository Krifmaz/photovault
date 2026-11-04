package app.photovault.android.ui.backup

import androidx.compose.runtime.Immutable
import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import app.photovault.android.backup.BackupPhase
import app.photovault.android.backup.BackupReport
import app.photovault.android.backup.BackupScheduler
import app.photovault.android.backup.BackupSettings
import app.photovault.android.backup.BackupSettingsStore
import app.photovault.android.backup.LocalFolder
import app.photovault.android.backup.MediaAccess
import app.photovault.android.backup.MediaSource
import app.photovault.android.backup.RunState
import app.photovault.android.data.CoreRepository
import app.photovault.android.data.UploadStatus
import kotlinx.coroutines.CoroutineDispatcher
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.SharingStarted
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.flow.distinctUntilChanged
import kotlinx.coroutines.flow.flow
import kotlinx.coroutines.flow.flowOn
import kotlinx.coroutines.flow.stateIn
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

@Immutable
data class FolderRow(val id: String, val name: String, val count: Int, val selected: Boolean)

@Immutable
data class BackupUiState(
    val loaded: Boolean = false,
    val signedIn: Boolean = false,
    val settings: BackupSettings = BackupSettings(),
    val access: MediaAccess = MediaAccess.None,
    val folders: List<FolderRow> = emptyList(),
    val phase: BackupPhase = BackupPhase.Idle,
    val runState: RunState = RunState.Off,
    val queue: UploadStatus? = null,
    val last: BackupReport? = null,
)

/** What the device and account look like; reloaded on resume and after a permission answer. */
private data class Library(val signedIn: Boolean, val access: MediaAccess, val folders: List<LocalFolder>)

class BackupViewModel(
    private val core: CoreRepository,
    private val store: BackupSettingsStore,
    private val media: MediaSource,
    private val scheduler: BackupScheduler,
    phase: StateFlow<BackupPhase>,
    private val io: CoroutineDispatcher = Dispatchers.IO,
    pollMs: Long = 1_500L,
) : ViewModel() {

    private val library = MutableStateFlow<Library?>(null)

    /** Core's upload queue, read while the screen is visible. */
    private val queue: Flow<UploadStatus?> = flow {
        while (true) {
            emit(if (core.currentAccount() != null) core.uploadStatus() else null)
            delay(pollMs)
        }
    }.flowOn(io).distinctUntilChanged()

    private val head = combine(store.settings, store.lastReport, phase, scheduler.runState) { s, last, p, run ->
        BackupUiState(settings = s, last = last, phase = p, runState = run)
    }

    val state: StateFlow<BackupUiState> = combine(head, library, queue) { h, lib, q ->
        if (lib == null) {
            h.copy(queue = q)
        } else {
            h.copy(
                loaded = true,
                signedIn = lib.signedIn,
                access = lib.access,
                folders = lib.folders.map { f ->
                    FolderRow(f.id, f.name, f.count, h.settings.folders?.contains(f.id) ?: f.isCamera)
                },
                queue = q,
            )
        }
    }.stateIn(viewModelScope, SharingStarted.WhileSubscribed(5_000), BackupUiState())

    init {
        refresh()
    }

    fun refresh() {
        viewModelScope.launch {
            library.value = withContext(io) {
                val access = media.access()
                Library(core.currentAccount() != null, access, if (access == MediaAccess.None) emptyList() else media.folders())
            }
        }
    }

    /** After the permission dialog: reload, and turn backup on if that is what the request was for. */
    fun onPermissionsResult(enableAfter: Boolean) {
        viewModelScope.launch {
            val access = withContext(io) { media.access() }
            refresh()
            if (enableAfter && access != MediaAccess.None) setEnabled(true)
        }
    }

    fun setEnabled(on: Boolean) = change(constraintsChanged = true) { it.copy(enabled = on) }

    fun setWifiOnly(on: Boolean) = change(constraintsChanged = true) { it.copy(wifiOnly = on) }

    fun setChargingOnly(on: Boolean) = change(constraintsChanged = true) { it.copy(chargingOnly = on) }

    fun setIncludeVideos(on: Boolean) = change(constraintsChanged = false) { it.copy(includeVideos = on) }

    fun toggleFolder(id: String) {
        val folders = library.value?.folders ?: return
        change(constraintsChanged = false) { s ->
            val current = s.folders ?: folders.filter { it.isCamera }.mapTo(HashSet()) { it.id }
            s.copy(folders = if (id in current) current - id else current + id)
        }
    }

    fun backUpNow() {
        viewModelScope.launch(io) { scheduler.runNow(store.settings.value) }
    }

    private fun change(constraintsChanged: Boolean, edit: (BackupSettings) -> BackupSettings) {
        viewModelScope.launch(io) {
            val before = store.settings.value
            val after = store.update(edit)
            if (after != before) scheduler.apply(after, changed = constraintsChanged || before.enabled != after.enabled)
        }
    }
}
