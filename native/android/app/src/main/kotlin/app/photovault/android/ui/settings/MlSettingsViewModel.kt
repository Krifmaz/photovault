package app.photovault.android.ui.settings

import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import app.photovault.android.data.CoreError
import app.photovault.android.data.ml.ChargingSetting
import app.photovault.android.data.ml.MlRepository
import app.photovault.android.data.ml.MlScheduler
import app.photovault.android.data.ml.MlStatus
import kotlinx.coroutines.CoroutineDispatcher
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.SharingStarted
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.flow.flow
import kotlinx.coroutines.flow.flowOn
import kotlinx.coroutines.flow.merge
import kotlinx.coroutines.flow.onStart
import kotlinx.coroutines.flow.stateIn
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

/** Everything the machine learning settings screen shows. */
data class MlSettingsState(
    val status: MlStatus? = null,
    val onlyWhileCharging: Boolean = true,
    /** A switch change is in flight (the remote flag is written through core). */
    val busy: Boolean = false,
    /** The last switch change failed. */
    val error: String? = null,
)

/**
 * Machine learning settings: Ente's `faceSearchEnabled` switch (through core's `set_ml_enabled`), the
 * indexing status (polled from core's fast `ml_status` while the screen is visible), and "index only while
 * charging". Turning ML on (or changing a setting) schedules the background work, turning it off cancels it.
 */
class MlSettingsViewModel(
    private val ml: MlRepository,
    private val charging: ChargingSetting,
    private val scheduler: MlScheduler,
    private val io: CoroutineDispatcher = Dispatchers.IO,
    pollMs: Long = POLL_MS,
) : ViewModel() {
    private val local = MutableStateFlow(MlSettingsState())
    private val updates = MutableSharedFlow<MlStatus>(extraBufferCapacity = 1)

    /** Polls only while collected (the screen is started), so nothing runs once it is gone. */
    private val polled = flow {
        while (true) {
            emit(ml.mlStatus())
            delay(pollMs)
        }
    }.flowOn(io)

    /** The newest status: a poll, or the answer of a switch change. */
    private val status = merge(polled, updates).onStart<MlStatus?> { emit(null) }

    val state: StateFlow<MlSettingsState> = combine(local, status) { l, s -> if (s == null) l else l.copy(status = s) }
        .stateIn(viewModelScope, SharingStarted.WhileSubscribed(STOP_TIMEOUT_MS), MlSettingsState())

    init {
        viewModelScope.launch {
            val value = withContext(io) { charging.get() }
            local.update { it.copy(onlyWhileCharging = value) }
        }
    }

    fun setEnabled(enabled: Boolean) {
        if (local.value.busy) return
        viewModelScope.launch {
            local.update { it.copy(busy = true, error = null) }
            try {
                val result = ml.setMlEnabled(enabled)
                local.update { it.copy(status = result) }
                updates.tryEmit(result)
                if (result.enabled) scheduler.reschedule() else scheduler.cancel()
            } catch (e: CoreError) {
                local.update { it.copy(error = e.message) }
            } finally {
                local.update { it.copy(busy = false) }
            }
        }
    }

    fun setOnlyWhileCharging(value: Boolean) {
        local.update { it.copy(onlyWhileCharging = value) }
        viewModelScope.launch {
            withContext(io) { charging.set(value) }
            if (withContext(io) { ml.mlStatus() }.enabled) scheduler.reschedule()
        }
    }

    companion object {
        const val POLL_MS = 1_000L
        private const val STOP_TIMEOUT_MS = 2_000L
    }
}

/** The status lines, as data, so the wording is tested without Compose. */
sealed interface MlStatusLine {
    /** This build cannot run the models here; results from other devices still appear. */
    data object NoInference : MlStatusLine
    data class DownloadingModels(val doneMb: Long, val totalMb: Long) : MlStatusLine
    data class Indexed(val done: Long, val total: Long) : MlStatusLine
    data object Running : MlStatusLine
    data class Problem(val message: String) : MlStatusLine
}

/** What to show for [status] (nothing while ML is off). */
fun statusLines(status: MlStatus?): List<MlStatusLine> {
    if (status == null || !status.enabled) return emptyList()
    val out = ArrayList<MlStatusLine>(4)
    if (!status.inferenceAvailable) out += MlStatusLine.NoInference
    if (status.inferenceAvailable && !status.modelsReady && status.modelBytesTotal > 0) {
        out += MlStatusLine.DownloadingModels(mb(status.modelBytesDone), mb(status.modelBytesTotal))
    }
    if (status.filesTotal > 0) out += MlStatusLine.Indexed(status.filesIndexed, status.filesTotal)
    if (status.running) out += MlStatusLine.Running
    status.lastError?.takeIf { it.isNotBlank() }?.let { out += MlStatusLine.Problem(it) }
    return out
}

private fun mb(bytes: Long): Long = (bytes + 512 * 1024) / (1024 * 1024)
