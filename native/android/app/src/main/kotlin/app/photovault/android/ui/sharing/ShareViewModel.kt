package app.photovault.android.ui.sharing

import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import app.photovault.android.data.CoreError
import app.photovault.android.data.albums.AlbumDetails
import app.photovault.android.data.albums.AlbumsRepository
import app.photovault.android.data.albums.PublicLinkUpdate
import app.photovault.android.data.albums.ShareRole
import kotlinx.coroutines.CoroutineDispatcher
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

/** How long a public link stays valid, from now. */
enum class LinkExpiry(val durationUs: Long) {
    Never(0),
    Hour(3_600_000_000L),
    Day(86_400_000_000L),
    Week(7 * 86_400_000_000L),
    Month(30 * 86_400_000_000L),
    Year(365 * 86_400_000_000L),
}

/** Device limits Ente's apps offer for a link (0 = no limit). */
val DeviceLimits: List<Int> = listOf(0, 2, 5, 10, 25, 50)

data class ShareState(
    val details: AlbumDetails? = null,
    val loaded: Boolean = false,
    val busy: Boolean = false,
    val error: String? = null,
    /** Set after a successful add, so the screen clears the email field. */
    val addedEmail: String? = null,
)

/** Who an album is shared with, and its public link. */
class ShareViewModel(
    private val albums: AlbumsRepository,
    val collectionId: Long,
    private val io: CoroutineDispatcher = Dispatchers.IO,
    private val nowUs: () -> Long = { System.currentTimeMillis() * 1000L },
) : ViewModel() {

    private val _state = MutableStateFlow(ShareState())
    val state: StateFlow<ShareState> = _state.asStateFlow()

    init {
        viewModelScope.launch { reload() }
    }

    private suspend fun reload() {
        val d = withContext(io) { albums.albumDetails(collectionId) }
        _state.update { it.copy(details = d, loaded = true) }
    }

    fun share(email: String, role: ShareRole) {
        val e = email.trim()
        if (e.isEmpty()) return
        act {
            albums.shareAlbum(collectionId, e, role)
            _state.update { it.copy(addedEmail = e) }
        }
    }

    fun addedEmailConsumed() = _state.update { it.copy(addedEmail = null) }

    /** Change someone's role: Ente re-shares with the new role. */
    fun changeRole(email: String, role: ShareRole) = act { albums.shareAlbum(collectionId, email, role) }

    fun unshare(email: String) = act { albums.unshareAlbum(collectionId, email) }

    fun createLink() = act { albums.createPublicLink(collectionId) }

    fun removeLink() = act { albums.deletePublicLink(collectionId) }

    fun setExpiry(expiry: LinkExpiry) =
        update(PublicLinkUpdate(validTillUs = if (expiry == LinkExpiry.Never) 0 else nowUs() + expiry.durationUs))

    fun setDeviceLimit(limit: Int) = update(PublicLinkUpdate(deviceLimit = limit))

    fun setDownloads(allowed: Boolean) = update(PublicLinkUpdate(enableDownload = allowed))

    fun setCollect(allowed: Boolean) = update(PublicLinkUpdate(enableCollect = allowed))

    fun setPassword(password: String) {
        if (password.isEmpty()) return
        update(PublicLinkUpdate(password = password))
    }

    fun removePassword() = update(PublicLinkUpdate(password = ""))

    private fun update(u: PublicLinkUpdate) = act { albums.updatePublicLink(collectionId, u) }

    fun clearError() = _state.update { it.copy(error = null) }

    private fun act(action: suspend () -> Unit) {
        if (_state.value.busy) return
        viewModelScope.launch {
            _state.update { it.copy(busy = true, error = null) }
            try {
                action()
                reload()
            } catch (e: CoreError) {
                _state.update { it.copy(error = e.message) }
            } finally {
                _state.update { it.copy(busy = false) }
            }
        }
    }
}
