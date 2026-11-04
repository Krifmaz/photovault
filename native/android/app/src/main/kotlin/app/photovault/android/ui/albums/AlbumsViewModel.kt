package app.photovault.android.ui.albums

import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import app.photovault.android.data.CoreError
import app.photovault.android.data.CoreRepository
import app.photovault.android.data.albums.AlbumDetails
import app.photovault.android.data.albums.AlbumsRepository
import kotlinx.coroutines.CoroutineDispatcher
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.receiveAsFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

/** The Albums tab: the user's albums, albums others share with them, and the ways into Hidden and Trash. */
data class AlbumsState(
    val loaded: Boolean = false,
    val mine: List<AlbumDetails> = emptyList(),
    val shared: List<AlbumDetails> = emptyList(),
    val trashCount: Long = 0,
    val syncing: Boolean = false,
    val creating: Boolean = false,
    val error: String? = null,
    val demo: Boolean = false,
)

sealed interface AlbumsEvent {
    data class Open(val collectionId: Long) : AlbumsEvent
}

class AlbumsViewModel(
    private val albums: AlbumsRepository,
    private val core: CoreRepository,
    private val io: CoroutineDispatcher = Dispatchers.IO,
) : ViewModel() {

    private val _state = MutableStateFlow(AlbumsState(demo = albums.isDemo))
    val state: StateFlow<AlbumsState> = _state.asStateFlow()
    private val events = Channel<AlbumsEvent>(Channel.BUFFERED)
    val eventFlow: Flow<AlbumsEvent> = events.receiveAsFlow()
    private var syncJob: Job? = null

    init {
        refresh()
    }

    /** Re-reads core's local index (fast; after returning from an album or a sync). */
    fun refresh() {
        viewModelScope.launch { reload() }
    }

    private suspend fun reload() {
        val (all, trash) = withContext(io) { albums.albums() to albums.trashCount() }
        _state.update { s ->
            s.copy(
                loaded = true,
                mine = all.filter { it.owned },
                shared = all.filter { !it.owned },
                trashCount = trash,
            )
        }
    }

    /** Pull to refresh: sync the library with the server, then reload. */
    fun sync() {
        if (syncJob?.isActive == true) return
        syncJob = viewModelScope.launch {
            _state.update { it.copy(syncing = true, error = null) }
            try {
                core.sync(null)
            } catch (e: CoreError) {
                _state.update { it.copy(error = e.message) }
            } finally {
                reload()
                _state.update { it.copy(syncing = false) }
            }
        }
    }

    fun createAlbum(name: String) {
        if (name.isBlank() || _state.value.creating) return
        viewModelScope.launch {
            _state.update { it.copy(creating = true, error = null) }
            try {
                val c = albums.createAlbum(name.trim())
                reload()
                events.send(AlbumsEvent.Open(c.id))
            } catch (e: CoreError) {
                _state.update { it.copy(error = e.message) }
            } finally {
                _state.update { it.copy(creating = false) }
            }
        }
    }

    fun clearError() = _state.update { it.copy(error = null) }
}
