package app.photovault.android.ui.people

import androidx.compose.runtime.Immutable
import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import app.photovault.android.data.CoreError
import app.photovault.android.data.ml.MlRepository
import app.photovault.android.data.ml.Person
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

@Immutable
data class MergeState(
    val loaded: Boolean = false,
    val person: Person? = null,
    /** Everyone this person can be merged with: named people first, then unnamed clusters. */
    val candidates: List<Person> = emptyList(),
    val busy: Boolean = false,
    val error: String? = null,
)

/** "Merge with…": joins this person and another into one (core's `merge_people`). Based on Ente (ente.com). */
class MergePeopleViewModel(
    private val ml: MlRepository,
    private val personId: String,
    private val io: CoroutineDispatcher = Dispatchers.IO,
) : ViewModel() {

    private val _state = MutableStateFlow(MergeState())
    private val events = Channel<PersonDone>(Channel.BUFFERED)

    val state: StateFlow<MergeState> = _state.asStateFlow()
    val doneFlow: Flow<PersonDone> = events.receiveAsFlow()

    init {
        viewModelScope.launch {
            val (person, people) = withContext(io) {
                ml.person(personId) to ml.people(includeHidden = false, minFiles = 1)
            }
            _state.update { it.copy(loaded = true, person = person, candidates = people.filter { p -> p.id != personId }) }
        }
    }

    fun merge(other: Person) {
        val s = _state.value
        val current = s.person ?: return
        if (s.busy) return
        val (into, from) = direction(current, other)
        viewModelScope.launch {
            _state.update { it.copy(busy = true, error = null) }
            try {
                events.send(PersonDone(ml.mergePeople(intoId = into, fromId = from).id))
            } catch (e: CoreError) {
                _state.update { it.copy(error = e.message) }
            } finally {
                _state.update { it.copy(busy = false) }
            }
        }
    }

    companion object {
        /**
         * Which one survives: a named person keeps its name and id, so an unnamed cluster always goes into a
         * named person; between two named people (or two clusters) the one being viewed stays when it is named.
         * Returns (into, from).
         */
        fun direction(current: Person, other: Person): Pair<String, String> =
            if (current.name != null) current.id to other.id else other.id to current.id
    }
}
