package app.photovault.android.ui.memories

import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.rememberUpdatedState
import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import app.photovault.android.data.ml.Memory
import app.photovault.android.ui.timeline.TimelineStatus
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.distinctUntilChanged
import kotlinx.coroutines.flow.drop
import kotlinx.coroutines.flow.filter
import kotlinx.coroutines.flow.map
import kotlinx.coroutines.launch

/**
 * The memories strip above the timeline. Lives as long as the timeline's back stack entry: a new one (after
 * a sign-in) starts from an empty list, so another account's memories never show.
 */
class MemoriesViewModel(private val feed: MemoryFeed) : ViewModel() {
    val memories: StateFlow<List<Memory>> = feed.memories

    init {
        feed.clear()
        refresh()
    }

    /** Reloads after a library sync (new photos, new ML data from other devices, another day). */
    fun refresh() {
        viewModelScope.launch { feed.refresh() }
    }
}

/**
 * Calls [onSynced] each time a timeline sync finishes without error (not for the state at first composition).
 * Used to reload memories and to start background ML work after the app synced.
 */
@Composable
fun AfterSync(status: StateFlow<TimelineStatus>, onSynced: () -> Unit) {
    val callback by rememberUpdatedState(onSynced)
    LaunchedEffect(status) {
        status
            .map { it.syncing to it.syncError }
            .distinctUntilChanged()
            .drop(1)
            .filter { (syncing, error) -> !syncing && error == null }
            .collect { callback() }
    }
}
