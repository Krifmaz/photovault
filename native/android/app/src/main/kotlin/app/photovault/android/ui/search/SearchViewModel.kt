package app.photovault.android.ui.search

import androidx.compose.runtime.Immutable
import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import app.photovault.android.data.CoreError
import app.photovault.android.data.ml.MlRepository
import app.photovault.android.data.ml.SearchKind
import app.photovault.android.data.ml.SearchSuggestion
import kotlinx.coroutines.CoroutineDispatcher
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.FlowPreview
import kotlinx.coroutines.Job
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.collectLatest
import kotlinx.coroutines.flow.debounce
import kotlinx.coroutines.flow.distinctUntilChanged
import kotlinx.coroutines.flow.map
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

/** Suggestions of one kind, in core's order. */
@Immutable
data class SuggestionGroup(val kind: SearchKind, val items: List<SearchSuggestion>)

/** What the search screen shows before anything is typed (core's `search_discovery`, by kind). */
@Immutable
data class Discovery(
    val people: List<SearchSuggestion> = emptyList(),
    val places: List<SearchSuggestion> = emptyList(),
    val fileTypes: List<SearchSuggestion> = emptyList(),
    val albums: List<SearchSuggestion> = emptyList(),
    val dates: List<SearchSuggestion> = emptyList(),
    val magic: List<SearchSuggestion> = emptyList(),
) {
    val isEmpty: Boolean
        get() = people.isEmpty() && places.isEmpty() && fileTypes.isEmpty() && albums.isEmpty() && dates.isEmpty() && magic.isEmpty()

    companion object {
        fun of(list: List<SearchSuggestion>): Discovery {
            val by = list.groupBy { it.kind }
            return Discovery(
                people = by[SearchKind.Person].orEmpty(),
                places = by[SearchKind.Place].orEmpty(),
                fileTypes = by[SearchKind.FileType].orEmpty(),
                albums = by[SearchKind.Album].orEmpty(),
                dates = by[SearchKind.Date].orEmpty(),
                magic = by[SearchKind.Magic].orEmpty(),
            )
        }
    }
}

@Immutable
data class SearchState(
    /** The text in the field, exactly as typed. */
    val query: String = "",
    /** The trimmed query [groups] answer. */
    val answered: String = "",
    val groups: List<SuggestionGroup> = emptyList(),
    val discovery: Discovery = Discovery(),
    val discoveryLoaded: Boolean = false,
    val mlEnabled: Boolean = true,
    val mlBusy: Boolean = false,
    val error: String? = null,
) {
    /** A typed query whose suggestions have not arrived yet (the previous ones stay on screen meanwhile). */
    val searching: Boolean get() = query.isNotBlank() && query.trim() != answered

    /** Nothing matched what was typed. */
    val noResults: Boolean get() = query.isNotBlank() && !searching && groups.isEmpty()
}

/**
 * The search screen: suggestions for what is typed (core's `search_suggestions`, debounced, off the main
 * thread) and the discovery content shown before typing.
 */
@OptIn(FlowPreview::class)
class SearchViewModel(
    private val ml: MlRepository,
    private val io: CoroutineDispatcher = Dispatchers.IO,
    private val debounceMs: Long = DEBOUNCE_MS,
) : ViewModel() {

    private val query = MutableStateFlow("")
    private val _state = MutableStateFlow(SearchState())
    private var discoveryJob: Job? = null

    val state: StateFlow<SearchState> = _state.asStateFlow()

    init {
        refreshDiscovery()
        viewModelScope.launch {
            query.map { it.trim() }
                .distinctUntilChanged()
                .debounce { if (it.isEmpty()) 0L else debounceMs }
                .collectLatest { q ->
                    if (q.isEmpty()) {
                        _state.update { it.copy(answered = "", groups = emptyList()) }
                        return@collectLatest
                    }
                    val list = try {
                        withContext(io) { ml.searchSuggestions(q, LIMIT) }
                    } catch (e: CoreError) {
                        _state.update { it.copy(error = e.message) }
                        emptyList()
                    }
                    _state.update { it.copy(answered = q, groups = group(list)) }
                }
        }
    }

    fun onQuery(text: String) {
        _state.update { it.copy(query = text, error = null) }
        query.value = text
    }

    fun clear() = onQuery("")

    /**
     * What the search key opens: the best suggestion for the current text, or a magic search for it while the
     * suggestions are still on their way (or none matched). Null for an empty query.
     */
    fun submitTarget(): SearchSuggestion? {
        val s = _state.value
        val q = s.query.trim()
        if (q.isEmpty()) return null
        if (q == s.answered) s.groups.firstOrNull()?.items?.firstOrNull()?.let { return it }
        return SearchSuggestion(SearchKind.Magic, q, q, 0, null, null)
    }

    /** Reloads the discovery content and the ML switch (on open, and after people changed). */
    fun refreshDiscovery() {
        if (discoveryJob?.isActive == true) return
        discoveryJob = viewModelScope.launch {
            try {
                val (status, list) = withContext(io) { ml.mlStatus() to ml.searchDiscovery() }
                _state.update { it.copy(discovery = Discovery.of(list), discoveryLoaded = true, mlEnabled = status.enabled) }
            } catch (e: CoreError) {
                _state.update { it.copy(discoveryLoaded = true, error = e.message) }
            }
        }
    }

    /** Turns on faces and magic search (Ente's `faceSearchEnabled`, shared with the Ente apps). */
    fun enableMl() {
        if (_state.value.mlBusy) return
        viewModelScope.launch {
            _state.update { it.copy(mlBusy = true, error = null) }
            try {
                val status = ml.setMlEnabled(true)
                _state.update { it.copy(mlEnabled = status.enabled) }
                // pull what other devices already indexed, so people show up right away
                runCatching { ml.syncMl(null) }
            } catch (e: CoreError) {
                _state.update { it.copy(error = e.message) }
            } finally {
                _state.update { it.copy(mlBusy = false) }
            }
            discoveryJob?.join()
            refreshDiscovery()
        }
    }

    companion object {
        const val DEBOUNCE_MS = 250L
        const val LIMIT = 40

        /** Section order on screen. */
        val ORDER: List<SearchKind> = listOf(
            SearchKind.Person,
            SearchKind.Album,
            SearchKind.Place,
            SearchKind.Date,
            SearchKind.FileType,
            SearchKind.FileName,
            SearchKind.Caption,
            SearchKind.Magic,
        )

        fun group(list: List<SearchSuggestion>): List<SuggestionGroup> {
            val by = list.groupBy { it.kind }
            return ORDER.mapNotNull { kind -> by[kind]?.let { SuggestionGroup(kind, it) } }
        }
    }
}
