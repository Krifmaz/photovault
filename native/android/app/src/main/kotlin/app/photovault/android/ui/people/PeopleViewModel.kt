package app.photovault.android.ui.people

import androidx.compose.runtime.Immutable
import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import app.photovault.android.data.CoreError
import app.photovault.android.data.ml.MlRepository
import app.photovault.android.data.ml.Person
import kotlinx.coroutines.CoroutineDispatcher
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

@Immutable
data class PeopleState(
    val loaded: Boolean = false,
    /** Named people first (most photos first), then unnamed clusters; hidden ones only with [showHidden]. */
    val people: List<Person> = emptyList(),
    val showHidden: Boolean = false,
    val mlEnabled: Boolean = true,
    val error: String? = null,
)

/** All people (core's `people`). */
class PeopleViewModel(
    private val ml: MlRepository,
    private val io: CoroutineDispatcher = Dispatchers.IO,
) : ViewModel() {

    private val _state = MutableStateFlow(PeopleState())
    val state: StateFlow<PeopleState> = _state.asStateFlow()
    private var job: Job? = null

    init {
        refresh()
    }

    /** Reloads (on open and when coming back from a person, who may have been named, merged or hidden). */
    fun refresh() {
        job?.cancel()
        val hidden = _state.value.showHidden
        job = viewModelScope.launch {
            try {
                val (enabled, people) = withContext(io) { ml.mlStatus().enabled to ml.people(hidden, MIN_FILES) }
                _state.update { it.copy(loaded = true, people = people, mlEnabled = enabled, error = null) }
            } catch (e: CoreError) {
                _state.update { it.copy(loaded = true, error = e.message) }
            }
        }
    }

    fun setShowHidden(show: Boolean) {
        if (_state.value.showHidden == show) return
        _state.update { it.copy(showHidden = show) }
        refresh()
    }

    companion object {
        /** Unnamed clusters with fewer photos are left out (Ente hides tiny clusters too). */
        const val MIN_FILES = 2
    }
}
