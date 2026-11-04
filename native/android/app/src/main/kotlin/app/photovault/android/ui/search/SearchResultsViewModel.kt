package app.photovault.android.ui.search

import androidx.compose.runtime.Immutable
import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import app.photovault.android.data.CoreError
import app.photovault.android.data.TimelineItem
import app.photovault.android.data.ml.MlRepository
import app.photovault.android.data.ml.SearchKind
import app.photovault.android.ui.timeline.GridModel
import app.photovault.android.ui.timeline.TimelineLabels
import kotlinx.coroutines.CoroutineDispatcher
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

@Immutable
data class ResultsState(
    val loaded: Boolean = false,
    val grid: GridModel = GridModel.Empty,
    val count: Int = 0,
    val error: String? = null,
)

/** The photos of one suggestion (core's `search_results`), in the timeline's grid. */
class SearchResultsViewModel(
    private val ml: MlRepository,
    val kind: SearchKind,
    val key: String,
    private val labels: TimelineLabels,
    private val compute: CoroutineDispatcher = Dispatchers.Default,
) : ViewModel() {

    /** What the viewer opened from these results pages through. */
    val source: String = PhotoSource.search(kind, key)

    private val _state = MutableStateFlow(ResultsState())
    private var items: List<TimelineItem> = emptyList()
    val state: StateFlow<ResultsState> = _state.asStateFlow()

    init {
        load()
    }

    fun load() {
        viewModelScope.launch {
            _state.update { it.copy(error = null) }
            try {
                // core's async call runs on core's runtime; the grid is built off the main thread
                val items = ml.searchResults(kind, key)
                val grid = withContext(compute) { photoGrid(items, grouped = kind != SearchKind.Magic, labels) }
                this@SearchResultsViewModel.items = items
                ResultLists.put(source, items)
                _state.update { ResultsState(loaded = true, grid = grid, count = items.size) }
            } catch (e: CoreError) {
                _state.update { it.copy(loaded = true, error = e.message) }
            }
        }
    }

    /** Before opening the viewer: make sure it pages through this list. */
    fun prepareViewer() {
        ResultLists.put(source, items)
    }
}
