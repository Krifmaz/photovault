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
import java.util.Locale

@Immutable
data class NameState(
    val loaded: Boolean = false,
    val person: Person? = null,
    val name: String = "",
    /** Other named people whose name matches what is typed: picking one merges into them. */
    val matches: List<Person> = emptyList(),
    val saving: Boolean = false,
    val error: String? = null,
)

/** The person now shown for the edited one (a named cluster or a merge gets another id). */
data class PersonDone(val personId: String)

/**
 * Name a cluster or rename a person. Like Ente's apps, choosing (or typing exactly) the name of another
 * person merges this one into them instead of making a second person with the same name.
 * Based on Ente (ente.com).
 */
class NamePersonViewModel(
    private val ml: MlRepository,
    private val personId: String,
    private val io: CoroutineDispatcher = Dispatchers.IO,
) : ViewModel() {

    private val _state = MutableStateFlow(NameState())
    private val events = Channel<PersonDone>(Channel.BUFFERED)
    private var named: List<Person> = emptyList()

    val state: StateFlow<NameState> = _state.asStateFlow()
    val doneFlow: Flow<PersonDone> = events.receiveAsFlow()

    init {
        viewModelScope.launch {
            val (person, people) = withContext(io) {
                ml.person(personId) to ml.people(includeHidden = true, minFiles = 1)
            }
            named = people.filter { it.name != null && it.id != personId }
            _state.update {
                // keep what the user may already have typed
                val name = it.name.ifEmpty { person?.name.orEmpty() }
                it.copy(loaded = true, person = person, name = name, matches = matching(name))
            }
        }
    }

    fun onName(text: String) {
        _state.update { it.copy(name = text, matches = matching(text), error = null) }
    }

    private fun matching(text: String): List<Person> {
        val q = text.trim().lowercase(Locale.getDefault())
        if (q.isEmpty()) return emptyList()
        return named.filter { it.name!!.lowercase(Locale.getDefault()).contains(q) }.take(MAX_MATCHES)
    }

    fun save() {
        val s = _state.value
        val name = s.name.trim()
        if (name.isEmpty() || s.saving) return
        if (name == s.person?.name) {
            viewModelScope.launch { events.send(PersonDone(personId)) }
            return
        }
        val same = named.firstOrNull { it.name.equals(name, ignoreCase = true) }
        if (same != null) {
            mergeInto(same)
            return
        }
        submit { ml.namePerson(personId, name).id }
    }

    /** Merges this person into [other] (an existing name). */
    fun mergeInto(other: Person) {
        if (_state.value.saving) return
        submit { ml.mergePeople(intoId = other.id, fromId = personId).id }
    }

    private fun submit(block: suspend () -> String) {
        viewModelScope.launch {
            _state.update { it.copy(saving = true, error = null) }
            try {
                events.send(PersonDone(block()))
            } catch (e: CoreError) {
                _state.update { it.copy(error = e.message) }
            } finally {
                _state.update { it.copy(saving = false) }
            }
        }
    }

    companion object {
        const val MAX_MATCHES = 5
    }
}
