@file:OptIn(ExperimentalMaterial3Api::class)

package app.photovault.android.ui.search

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.grid.rememberLazyGridState
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBar
import androidx.compose.material3.TopAppBarDefaults
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.res.pluralStringResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import app.photovault.android.R
import app.photovault.android.data.ml.SearchKind
import app.photovault.android.ui.components.LinkButton
import app.photovault.android.ui.theme.Spacing
import app.photovault.android.ui.theme.palette
import kotlinx.coroutines.flow.Flow

/** The photos of one search suggestion, in the timeline's grid; a tap opens the viewer over this list. */
@Composable
fun SearchResultsScreen(
    vm: SearchResultsViewModel,
    title: String,
    returnIndex: Flow<Int?>,
    onReturnConsumed: () -> Unit,
    onBack: () -> Unit,
    onOpen: (index: Int, fileId: Long) -> Unit,
) {
    val state by vm.state.collectAsStateWithLifecycle()
    val p = MaterialTheme.palette
    val gridState = rememberLazyGridState()
    ScrollToReturned(returnIndex, onReturnConsumed, { vm.state.value.grid }, gridState)

    Scaffold(
        containerColor = p.bg,
        topBar = {
            TopAppBar(
                colors = TopAppBarDefaults.topAppBarColors(containerColor = p.bg, titleContentColor = p.text),
                navigationIcon = {
                    IconButton(onClick = onBack) {
                        Icon(painterResource(R.drawable.ic_arrow_back), contentDescription = stringResource(R.string.back), tint = p.text)
                    }
                },
                title = {
                    Column {
                        Text(title, style = MaterialTheme.typography.titleLarge, maxLines = 1, overflow = TextOverflow.Ellipsis)
                        if (state.loaded && state.error == null) {
                            val sub = if (vm.kind == SearchKind.Magic) stringResource(R.string.search_magic_row)
                            else pluralStringResource(R.plurals.search_photos, state.count, state.count)
                            Text(sub, style = MaterialTheme.typography.bodySmall, color = p.muted, maxLines = 1)
                        }
                    }
                },
            )
        },
    ) { padding ->
        when {
            !state.loaded -> Box(Modifier.fillMaxSize().padding(padding), contentAlignment = Alignment.Center) {
                CircularProgressIndicator(color = p.accent2)
            }
            state.error != null || state.count == 0 -> Column(
                Modifier.fillMaxSize().padding(padding).padding(Spacing.section),
                horizontalAlignment = Alignment.CenterHorizontally,
                verticalArrangement = Arrangement.spacedBy(Spacing.md, Alignment.CenterVertically),
            ) {
                Text(
                    state.error ?: stringResource(if (vm.kind == SearchKind.Magic) R.string.search_magic_none else R.string.search_results_none),
                    style = MaterialTheme.typography.bodyMedium,
                    color = p.muted,
                    textAlign = TextAlign.Center,
                )
                if (state.error != null) LinkButton(stringResource(R.string.search_retry), onClick = vm::load)
            }
            else -> ResultsGrid(
                model = state.grid,
                state = gridState,
                contentPadding = padding,
                onOpen = { index, fileId ->
                    vm.prepareViewer()
                    onOpen(index, fileId)
                },
            )
        }
    }
}
