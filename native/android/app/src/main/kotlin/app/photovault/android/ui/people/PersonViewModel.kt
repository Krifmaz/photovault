package app.photovault.android.ui.people

import androidx.compose.runtime.Immutable
import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import app.photovault.android.data.CoreError
import app.photovault.android.data.TimelineItem
import app.photovault.android.data.ml.MlRepository
import app.photovault.android.data.ml.Person
import app.photovault.android.ui.search.PhotoSource
import app.photovault.android.ui.search.ResultLists
import app.photovault.android.ui.search.photoGrid
import app.photovault.android.ui.timeline.GridModel
import app.photovault.android.ui.timeline.TimelineLabels
import kotlinx.coroutines.CoroutineDispatcher
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
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
data class PersonState(
    val personId: String,
    val loaded: Boolean = false,
    /** Null once loaded = this person no longer exists (merged or removed elsewhere). */
    val person: Person? = null,
    val grid: GridModel = GridModel.Empty,
    val suggestionCount: Int = 0,
    /** "Remove photos": picking the photos that are not this person. */
    val selecting: Boolean = false,
    val selected: Set<Long> = emptySet(),
    val busy: Boolean = false,
    val error: String? = null,
)

sealed interface PersonEvent {
    /** The person is gone (name removed, or every photo removed): leave the screen. */
    data object Gone : PersonEvent
}

/** One person or unnamed cluster: cover face, name, photos and the actions on them. */
class PersonViewModel(
    private val ml: MlRepository,
    personId: String,
    private val labels: TimelineLabels,
    private val io: CoroutineDispatcher = Dispatchers.IO,
    private val compute: CoroutineDispatcher = Dispatchers.Default,
) : ViewModel() {

    private val _state = MutableStateFlow(PersonState(personId))
    private val events = Channel<PersonEvent>(Channel.BUFFERED)
    private var items: List<TimelineItem> = emptyList()
    private var loadJob: Job? = null

    val state: StateFlow<PersonState> = _state.asStateFlow()
    val eventFlow: Flow<PersonEvent> = events.receiveAsFlow()

    val personId: String get() = _state.value.personId

    /** What the viewer opened from this person pages through. */
    val source: String get() = PhotoSource.person(personId)

    init {
        refresh()
    }

    fun refresh() {
        loadJob?.cancel()
        val id = personId
        loadJob = viewModelScope.launch {
            try {
                val (person, files, suggestions) = withContext(io) {
                    Triple(ml.person(id), ml.personFiles(id), ml.personSuggestions(id).size)
                }
                val grid = withContext(compute) { photoGrid(files, grouped = true, labels) }
                items = files
                ResultLists.put(PhotoSource.person(id), files)
                _state.update {
                    if (it.personId != id) it
                    else it.copy(loaded = true, person = person, grid = grid, suggestionCount = suggestions, error = null)
                }
            } catch (e: CoreError) {
                _state.update { it.copy(loaded = true, error = e.message) }
            }
        }
    }

    /** Naming a cluster or merging gives the person another id: show that one from now on. */
    fun onPersonChanged(newId: String) {
        if (newId == personId) {
            refresh()
            return
        }
        _state.update { PersonState(newId, loaded = false, person = it.person) }
        refresh()
    }

    fun setHidden(hidden: Boolean) = act { ml.setPersonHidden(personId, hidden) }

    /** "Remove name": deletes the person (its `cgroup`); the faces become an unnamed cluster again. */
    fun removeName() {
        if (_state.value.person?.name == null) return
        act(refreshAfter = false) {
            ml.deletePerson(personId)
            events.send(PersonEvent.Gone)
        }
    }

    /** Uses a face on [fileId] as the avatar. */
    fun useAsCover(fileId: Long) = act {
        val id = personId
        val face = withContext(io) { ml.fileFaces(fileId).firstOrNull { it.personId == id } }
            ?: return@act
        ml.setPersonCover(id, face.face.faceId)
    }

    fun startSelecting() = _state.update { it.copy(selecting = true, selected = emptySet()) }

    fun cancelSelecting() = _state.update { it.copy(selecting = false, selected = emptySet()) }

    fun toggle(fileId: Long) = _state.update {
        if (!it.selecting) it
        else it.copy(selected = if (fileId in it.selected) it.selected - fileId else it.selected + fileId)
    }

    /** Removes this person's faces on the selected photos (they are rejected for this person). */
    fun removeSelected() {
        val chosen = _state.value.selected
        if (chosen.isEmpty()) return
        act {
            val id = personId
            val faceIds = withContext(io) {
                chosen.flatMap { file -> ml.fileFaces(file).filter { it.personId == id }.map { it.face.faceId } }
            }
            if (faceIds.isNotEmpty()) ml.removeFacesFromPerson(id, faceIds)
            _state.update { it.copy(selecting = false, selected = emptySet()) }
            val left = withContext(io) { ml.person(id) }
            if (left == null) events.send(PersonEvent.Gone)
        }
    }

    /** Before opening the viewer: make sure it pages through this person's photos. */
    fun prepareViewer() {
        ResultLists.put(source, items)
    }

    private fun act(refreshAfter: Boolean = true, block: suspend () -> Unit) {
        if (_state.value.busy) return
        viewModelScope.launch {
            _state.update { it.copy(busy = true, error = null) }
            try {
                block()
                if (refreshAfter) refresh()
            } catch (e: CoreError) {
                _state.update { it.copy(error = e.message) }
            } finally {
                _state.update { it.copy(busy = false) }
            }
        }
    }
}
