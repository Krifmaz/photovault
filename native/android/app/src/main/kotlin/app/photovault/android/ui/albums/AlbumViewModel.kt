package app.photovault.android.ui.albums

import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import app.photovault.android.data.CoreError
import app.photovault.android.data.TimelineItem
import app.photovault.android.data.TimelineStore
import app.photovault.android.data.albums.AlbumDetails
import app.photovault.android.data.albums.AlbumsRepository
import kotlinx.coroutines.CoroutineDispatcher
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.receiveAsFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

data class AlbumState(
    val details: AlbumDetails? = null,
    val loaded: Boolean = false,
    val selection: Set<Long> = emptySet(),
    val busy: Boolean = false,
    val error: String? = null,
)

sealed interface AlbumEvent {
    /** The album was deleted or left: close the screen. */
    data object Gone : AlbumEvent

    /** Files were moved elsewhere (trash, hidden, another album); [count] for the confirmation text. */
    data class Done(val kind: DoneKind, val count: Int) : AlbumEvent
}

enum class DoneKind { Added, Removed, Trashed, Hidden, Unhidden }

/** One album: its photos (over an album [TimelineStore], shared with the viewer), selection and actions. */
class AlbumViewModel(
    private val albums: AlbumsRepository,
    private val store: TimelineStore,
    val collectionId: Long,
    private val io: CoroutineDispatcher = Dispatchers.IO,
) : ViewModel() {

    private val _state = MutableStateFlow(AlbumState())
    val state: StateFlow<AlbumState> = _state.asStateFlow()
    val items: StateFlow<List<TimelineItem>> = store.items
    private val events = Channel<AlbumEvent>(Channel.BUFFERED)
    val eventFlow: Flow<AlbumEvent> = events.receiveAsFlow()

    init {
        refresh()
    }

    fun refresh() {
        viewModelScope.launch { reload() }
    }

    private suspend fun reload() {
        val details = withContext(io) { albums.albumDetails(collectionId) }
        store.refresh()
        val ids = store.items.value.mapTo(HashSet()) { it.id }
        _state.update { it.copy(details = details, loaded = true, selection = it.selection.filterTo(HashSet()) { id -> id in ids }) }
    }

    /** Called with the last visible index; loads ahead of the scroll. */
    fun onVisible(lastIndex: Int) {
        val want = lastIndex + TimelineStore.AHEAD
        if (want <= store.items.value.size) return
        viewModelScope.launch { store.ensureLoaded(want) }
    }

    // ---- selection ----

    fun toggle(fileId: Long) = _state.update {
        it.copy(selection = if (fileId in it.selection) it.selection - fileId else it.selection + fileId)
    }

    fun selectAll() = _state.update { s -> s.copy(selection = store.items.value.mapTo(HashSet()) { it.id }) }

    fun clearSelection() = _state.update { it.copy(selection = emptySet()) }

    fun clearError() = _state.update { it.copy(error = null) }

    /** Albums the selection can be added to (or unhidden/restored into): own or collaborative, not this one. */
    suspend fun targets(): List<AlbumDetails> = withContext(io) {
        albums.albums().filter { it.canAdd && it.id != collectionId && !it.isDefaultHidden }
    }

    // ---- album actions ----

    fun rename(name: String) = act { albums.renameAlbum(collectionId, name.trim()) }

    fun setHidden(hidden: Boolean) = act { albums.setAlbumHidden(collectionId, hidden) }

    fun delete(keepFiles: Boolean) = act(gone = true) { albums.deleteAlbum(collectionId, keepFiles) }

    fun leave() = act(gone = true) { albums.leaveAlbum(collectionId) }

    // ---- selection actions ----

    fun addSelectedTo(target: Long) = runOnSelection(DoneKind.Added) { albums.addToAlbum(target, it) }

    fun addSelectedToNew(name: String) = runOnSelection(DoneKind.Added) {
        val c = albums.createAlbum(name.trim())
        albums.addToAlbum(c.id, it)
    }

    fun removeSelected() = runOnSelection(DoneKind.Removed) { albums.removeFromAlbum(collectionId, it) }

    fun trashSelected() = runOnSelection(DoneKind.Trashed) { albums.moveToTrash(it) }

    fun hideSelected() = runOnSelection(DoneKind.Hidden) { albums.hideFiles(it) }

    fun unhideSelected(target: Long?) = runOnSelection(DoneKind.Unhidden) { albums.unhideFiles(it, target) }

    private fun runOnSelection(kind: DoneKind, action: suspend (List<Long>) -> Unit) {
        val ids = _state.value.selection.toList()
        if (ids.isEmpty()) return
        act {
            action(ids)
            _state.update { it.copy(selection = emptySet()) }
            events.send(AlbumEvent.Done(kind, ids.size))
        }
    }

    private fun act(gone: Boolean = false, action: suspend () -> Unit) {
        if (_state.value.busy) return
        viewModelScope.launch {
            _state.update { it.copy(busy = true, error = null) }
            try {
                action()
                if (gone) events.send(AlbumEvent.Gone) else reload()
            } catch (e: CoreError) {
                _state.update { it.copy(error = e.message) }
            } finally {
                _state.update { it.copy(busy = false) }
            }
        }
    }
}
