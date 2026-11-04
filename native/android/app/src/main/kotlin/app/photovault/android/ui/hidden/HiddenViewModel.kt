package app.photovault.android.ui.hidden

import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import app.photovault.android.data.CoreError
import app.photovault.android.data.albums.AlbumDetails
import app.photovault.android.data.albums.AlbumsRepository
import kotlinx.coroutines.CoroutineDispatcher
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

data class HiddenState(
    val unlocked: Boolean = false,
    val loaded: Boolean = false,
    val albums: List<AlbumDetails> = emptyList(),
    val busy: Boolean = false,
    val error: String? = null,
)

/**
 * Hidden albums and the hidden photos collection. Nothing is read until the screen has confirmed the
 * device lock ([unlocked]); the unlock lasts as long as this view model (until the screen is left).
 */
class HiddenViewModel(
    private val albums: AlbumsRepository,
    private val io: CoroutineDispatcher = Dispatchers.IO,
) : ViewModel() {

    private val _state = MutableStateFlow(HiddenState())
    val state: StateFlow<HiddenState> = _state.asStateFlow()

    fun unlocked() {
        if (_state.value.unlocked) return
        _state.update { it.copy(unlocked = true) }
        refresh()
    }

    fun refresh() {
        if (!_state.value.unlocked) return
        viewModelScope.launch {
            val list = withContext(io) { albums.hiddenAlbums() }
            _state.update { it.copy(albums = list, loaded = true) }
        }
    }

    fun unhide(collectionId: Long) {
        if (_state.value.busy) return
        viewModelScope.launch {
            _state.update { it.copy(busy = true, error = null) }
            try {
                albums.setAlbumHidden(collectionId, false)
                val list = withContext(io) { albums.hiddenAlbums() }
                _state.update { it.copy(albums = list) }
            } catch (e: CoreError) {
                _state.update { it.copy(error = e.message) }
            } finally {
                _state.update { it.copy(busy = false) }
            }
        }
    }

    fun clearError() = _state.update { it.copy(error = null) }
}
