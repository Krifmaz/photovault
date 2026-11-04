package app.photovault.android.ui.trash

import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import app.photovault.android.data.CoreError
import app.photovault.android.data.albums.AlbumDetails
import app.photovault.android.data.albums.AlbumsRepository
import app.photovault.android.data.albums.TrashEntry
import kotlinx.coroutines.CoroutineDispatcher
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

data class TrashState(
    val loaded: Boolean = false,
    val syncing: Boolean = false,
    val entries: List<TrashEntry> = emptyList(),
    val selection: Set<Long> = emptySet(),
    val busy: Boolean = false,
    val error: String? = null,
)

/** Whole days until the server deletes [entry] for good (0 = today). */
fun daysLeft(entry: TrashEntry, nowUs: Long): Int {
    val left = entry.deleteByUs - nowUs
    if (left <= 0) return 0
    return ((left + DAY_US - 1) / DAY_US).toInt()
}

private const val DAY_US = 86_400_000_000L

/** The trash: pulled from the server on open; restore, delete for good, empty. */
class TrashViewModel(
    private val albums: AlbumsRepository,
    private val io: CoroutineDispatcher = Dispatchers.IO,
    val nowUs: () -> Long = { System.currentTimeMillis() * 1000L },
) : ViewModel() {

    private val _state = MutableStateFlow(TrashState())
    val state: StateFlow<TrashState> = _state.asStateFlow()
    private var syncJob: Job? = null

    init {
        viewModelScope.launch { reload() }
        sync()
    }

    fun sync() {
        if (syncJob?.isActive == true) return
        syncJob = viewModelScope.launch {
            _state.update { it.copy(syncing = true, error = null) }
            try {
                albums.syncTrash()
            } catch (e: CoreError) {
                _state.update { it.copy(error = e.message) }
            } finally {
                reload()
                _state.update { it.copy(syncing = false) }
            }
        }
    }

    private suspend fun reload() {
        val entries = withContext(io) {
            val total = albums.trashCount()
            val out = ArrayList<TrashEntry>(total.coerceAtMost(PAGE.toLong() * 10).toInt())
            while (out.size < total) {
                val page = albums.trashPage(out.size.toLong(), PAGE)
                if (page.isEmpty()) break
                out += page
            }
            out.distinctBy { it.item.id }
        }
        val ids = entries.mapTo(HashSet()) { it.item.id }
        _state.update { s -> s.copy(loaded = true, entries = entries, selection = s.selection.filterTo(HashSet()) { it in ids }) }
    }

    fun toggle(fileId: Long) = _state.update {
        it.copy(selection = if (fileId in it.selection) it.selection - fileId else it.selection + fileId)
    }

    fun selectAll() = _state.update { s -> s.copy(selection = s.entries.mapTo(HashSet()) { it.item.id }) }

    fun clearSelection() = _state.update { it.copy(selection = emptySet()) }

    fun clearError() = _state.update { it.copy(error = null) }

    /** Albums a restore can go to (own albums; null in the picker = Uncategorized). */
    suspend fun restoreTargets(): List<AlbumDetails> = withContext(io) {
        albums.albums().filter { it.owned && !it.isDefaultHidden }
    }

    fun restoreSelected(target: Long?) = onSelection { albums.restoreFromTrash(it, target) }

    fun deleteSelected() = onSelection { albums.deleteFromTrash(it) }

    fun emptyTrash() = act { albums.emptyTrash() }

    private fun onSelection(action: suspend (List<Long>) -> Unit) {
        val ids = _state.value.selection.toList()
        if (ids.isEmpty()) return
        act {
            action(ids)
            _state.update { it.copy(selection = emptySet()) }
        }
    }

    private fun act(action: suspend () -> Unit) {
        if (_state.value.busy) return
        viewModelScope.launch {
            _state.update { it.copy(busy = true, error = null) }
            try {
                action()
            } catch (e: CoreError) {
                _state.update { it.copy(error = e.message) }
            } finally {
                reload()
                _state.update { it.copy(busy = false) }
            }
        }
    }

    companion object {
        const val PAGE = 500
    }
}
