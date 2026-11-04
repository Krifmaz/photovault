package app.photovault.android.ui.timeline

import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import app.photovault.android.data.Account
import app.photovault.android.data.CoreError
import app.photovault.android.data.CoreRepository
import app.photovault.android.data.SyncProgress
import app.photovault.android.data.TimelineItem
import app.photovault.android.data.TimelineStore
import kotlinx.coroutines.CoroutineDispatcher
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.Job
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.SharingStarted
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.flow.flowOn
import kotlinx.coroutines.flow.mapLatest
import kotlinx.coroutines.flow.receiveAsFlow
import kotlinx.coroutines.flow.stateIn
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

/** Sync and account state for the top bar. */
data class TimelineStatus(
    val account: Account? = null,
    val serverName: String? = null,
    val syncing: Boolean = false,
    val progress: SyncProgress? = null,
    val syncError: String? = null,
    val loaded: Boolean = false,
    val empty: Boolean = false,
    val demo: Boolean = false,
)

sealed interface TimelineEvent {
    data object SignedOut : TimelineEvent
    data object SwitchServer : TimelineEvent
}

@OptIn(ExperimentalCoroutinesApi::class)
class TimelineViewModel(
    private val core: CoreRepository,
    private val store: TimelineStore,
    private val labels: TimelineLabels,
    private val io: CoroutineDispatcher = Dispatchers.IO,
    compute: CoroutineDispatcher = Dispatchers.Default,
    autoSync: Boolean = true,
    /** Drops decrypted images held outside core (Coil's memory cache, viewer files); see AppGraph. */
    private val clearDecrypted: () -> Unit = {},
) : ViewModel() {

    private val zoomIndex = MutableStateFlow(DEFAULT_ZOOM_INDEX)
    private val _status = MutableStateFlow(TimelineStatus(demo = core.isDemo))
    private val events = Channel<TimelineEvent>(Channel.BUFFERED)
    private var syncJob: Job? = null

    // Last built (items, grouping) -> entries, so changing columns within a grouping reuses the list.
    private var cacheItems: List<TimelineItem>? = null
    private var cacheGrouping: Grouping? = null
    private var cacheResult: Pair<List<GridEntry>, IntArray>? = null

    val status: StateFlow<TimelineStatus> = _status.asStateFlow()
    val eventFlow: Flow<TimelineEvent> = events.receiveAsFlow()

    /** The grid model for the current zoom level, rebuilt off the main thread when items or grouping change. */
    val grid: StateFlow<GridModel> = combine(store.items, zoomIndex) { items, z -> items to z }
        .mapLatest { (items, z) ->
            val grouping = ZoomLevels[z].grouping
            val cached = cacheResult
            val built = if (cached != null && cacheItems === items && cacheGrouping == grouping) cached
            else buildGrid(items, grouping, labels).also {
                cacheItems = items
                cacheGrouping = grouping
                cacheResult = it
            }
            GridModel(built.first, z, built.second)
        }
        .flowOn(compute)
        .stateIn(viewModelScope, SharingStarted.Eagerly, GridModel.Empty)

    init {
        viewModelScope.launch {
            val account = withContext(io) { core.currentAccount() }
            val server = withContext(io) {
                val active = core.activeBaseUrl()
                core.savedServers().firstOrNull { it.publicUrl == account?.serverUrl || it.publicUrl == active || it.homeUrl == active }
            }
            _status.update { it.copy(account = account, serverName = server?.name?.ifBlank { null } ?: account?.serverUrl) }
            store.refresh()
            updateLoaded()
            if (autoSync) sync()
        }
    }

    private fun updateLoaded() {
        _status.update { it.copy(loaded = store.loaded.value, empty = store.loaded.value && store.total.value == 0L) }
    }

    /** Pulls changes from the server, then reloads the local timeline. */
    fun sync() {
        if (syncJob?.isActive == true) return
        syncJob = viewModelScope.launch {
            _status.update { it.copy(syncing = true, syncError = null) }
            try {
                val result = core.sync { p -> _status.update { it.copy(progress = p) } }
                _status.update { it.copy(progress = result) }
                store.refresh()
            } catch (e: CoreError) {
                _status.update { it.copy(syncError = e.message) }
                if (e is CoreError.NotLoggedIn || e is CoreError.Unauthorized) events.send(TimelineEvent.SignedOut)
            } finally {
                _status.update { it.copy(syncing = false) }
                updateLoaded()
            }
        }
    }

    /** Pinch: +1 = more columns (smaller cells), -1 = fewer. Returns whether the level changed. */
    fun zoomBy(step: Int): Boolean {
        val before = zoomIndex.value
        zoomIndex.value = (before + step).coerceIn(0, ZoomLevels.lastIndex)
        return zoomIndex.value != before
    }

    val currentZoomIndex: Int get() = zoomIndex.value

    /** Called with the last visible timeline index; loads the next chunk ahead of the scroll. */
    fun onVisible(lastTimelineIndex: Int) {
        val want = lastTimelineIndex + TimelineStore.AHEAD
        if (want <= store.items.value.size) return
        viewModelScope.launch { store.ensureLoaded(want) }
    }

    fun signOut() {
        viewModelScope.launch {
            runCatching { core.logout() }
            store.clear()
            clearDecrypted()
            events.send(TimelineEvent.SignedOut)
        }
    }

    fun switchServer() {
        viewModelScope.launch { events.send(TimelineEvent.SwitchServer) }
    }
}
