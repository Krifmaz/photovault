package app.photovault.android.ui.people

import androidx.compose.runtime.Immutable
import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import app.photovault.android.data.CoreError
import app.photovault.android.data.ml.MlRepository
import app.photovault.android.data.ml.Person
import kotlinx.coroutines.CoroutineDispatcher
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

/** One suggested cluster with a few of its photos. */
@Immutable
data class SuggestionCard(val cluster: Person, val sampleFileIds: List<Long>)

@Immutable
data class SuggestionsState(
    val loaded: Boolean = false,
    val person: Person? = null,
    val cards: List<SuggestionCard> = emptyList(),
    /** Clusters whose answer is being saved. */
    val answering: Set<String> = emptySet(),
    val accepted: Int = 0,
    val error: String? = null,
)

/**
 * "Review suggestions": unnamed clusters that look like this person, accepted or rejected one by one, like
 * Ente's cluster suggestions. Based on Ente (ente.com).
 */
class SuggestionsViewModel(
    private val ml: MlRepository,
    private val personId: String,
    private val io: CoroutineDispatcher = Dispatchers.IO,
) : ViewModel() {

    private val _state = MutableStateFlow(SuggestionsState())
    val state: StateFlow<SuggestionsState> = _state.asStateFlow()

    init {
        viewModelScope.launch {
            val (person, cards) = withContext(io) {
                ml.person(personId) to ml.personSuggestions(personId).map { c ->
                    SuggestionCard(c, ml.personFiles(c.id).take(SAMPLES).map { it.id })
                }
            }
            _state.update { it.copy(loaded = true, person = person, cards = cards) }
        }
    }

    /** Yes (the cluster joins this person) or no (it is not suggested for them again). */
    fun answer(card: SuggestionCard, accept: Boolean) {
        val id = card.cluster.id
        if (id in _state.value.answering) return
        viewModelScope.launch {
            _state.update { it.copy(answering = it.answering + id, error = null) }
            try {
                ml.answerSuggestion(personId, id, accept)
                _state.update {
                    it.copy(
                        cards = it.cards.filter { c -> c.cluster.id != id },
                        accepted = it.accepted + if (accept) 1 else 0,
                    )
                }
            } catch (e: CoreError) {
                _state.update { it.copy(error = e.message) }
            } finally {
                _state.update { it.copy(answering = it.answering - id) }
            }
        }
    }

    companion object {
        const val SAMPLES = 4
    }
}
