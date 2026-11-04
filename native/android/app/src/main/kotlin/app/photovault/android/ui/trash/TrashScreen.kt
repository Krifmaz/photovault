@file:OptIn(ExperimentalMaterial3Api::class)

package app.photovault.android.ui.trash

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.grid.GridCells
import androidx.compose.foundation.lazy.grid.LazyVerticalGrid
import androidx.compose.foundation.lazy.grid.items
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.Text
import androidx.compose.material3.pulltorefresh.PullToRefreshBox
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.res.pluralStringResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.style.TextAlign
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import app.photovault.android.R
import app.photovault.android.ui.albums.AlbumPickerDialog
import app.photovault.android.ui.albums.AlbumTopBar
import app.photovault.android.ui.albums.ConfirmDialog
import app.photovault.android.ui.albums.SelectablePhotoCell
import app.photovault.android.ui.theme.Spacing
import app.photovault.android.ui.theme.palette

private enum class TrashDialog { Restore, Delete, Empty }

/** The trash: what is in it, how long until each file goes, restore and delete for good. */
@Composable
fun TrashScreen(vm: TrashViewModel, onBack: () -> Unit) {
    val state by vm.state.collectAsStateWithLifecycle()
    val p = MaterialTheme.palette
    val snackbar = remember { SnackbarHostState() }
    var dialog by rememberSaveable { mutableStateOf<TrashDialog?>(null) }
    var more by remember { mutableStateOf(false) }
    val selecting = state.selection.isNotEmpty()
    val now = remember(state.entries) { vm.nowUs() }

    BackHandler(enabled = selecting) { vm.clearSelection() }
    LaunchedEffect(state.error) {
        state.error?.let {
            snackbar.showSnackbar(it)
            vm.clearError()
        }
    }

    Scaffold(
        containerColor = p.bg,
        snackbarHost = { SnackbarHost(snackbar) },
        topBar = {
            Column {
                if (selecting) {
                    AlbumTopBar(
                        title = pluralStringResource(R.plurals.selected_count, state.selection.size, state.selection.size),
                        subtitle = null,
                        onBack = vm::clearSelection,
                        navIcon = R.drawable.ic_close,
                    ) {
                        IconButton(onClick = { dialog = TrashDialog.Restore }, modifier = Modifier.testTag("trash_restore")) {
                            Icon(painterResource(R.drawable.ic_restore), contentDescription = stringResource(R.string.trash_restore), tint = p.text)
                        }
                        IconButton(onClick = { dialog = TrashDialog.Delete }, modifier = Modifier.testTag("trash_delete")) {
                            Icon(painterResource(R.drawable.ic_delete), contentDescription = stringResource(R.string.trash_delete_forever), tint = p.text)
                        }
                        IconButton(onClick = { more = true }) {
                            Icon(painterResource(R.drawable.ic_more_vert), contentDescription = stringResource(R.string.albums_more), tint = p.text)
                        }
                        DropdownMenu(expanded = more, onDismissRequest = { more = false }) {
                            DropdownMenuItem(text = { Text(stringResource(R.string.albums_select_all)) }, onClick = { more = false; vm.selectAll() })
                        }
                    }
                } else {
                    AlbumTopBar(
                        title = stringResource(R.string.trash_title),
                        subtitle = stringResource(R.string.trash_subtitle),
                        onBack = onBack,
                    ) {
                        if (state.entries.isNotEmpty()) {
                            IconButton(onClick = { dialog = TrashDialog.Empty }, modifier = Modifier.testTag("trash_empty")) {
                                Icon(painterResource(R.drawable.ic_delete), contentDescription = stringResource(R.string.trash_empty_action), tint = p.text)
                            }
                        }
                    }
                }
                if (state.busy) LinearProgressIndicator(Modifier.fillMaxWidth(), color = p.accent2, trackColor = p.panel)
            }
        },
    ) { padding ->
        PullToRefreshBox(
            isRefreshing = state.syncing && state.loaded,
            onRefresh = vm::sync,
            modifier = Modifier.fillMaxSize().padding(padding),
        ) {
            when {
                !state.loaded -> Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
                    CircularProgressIndicator(color = p.accent2)
                }
                state.entries.isEmpty() -> Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
                    Column(
                        Modifier.padding(Spacing.section),
                        horizontalAlignment = Alignment.CenterHorizontally,
                        verticalArrangement = Arrangement.spacedBy(Spacing.md),
                    ) {
                        Text(stringResource(R.string.trash_empty_title), style = MaterialTheme.typography.titleLarge, color = p.text)
                        Text(
                            if (state.syncing) stringResource(R.string.timeline_syncing) else stringResource(R.string.trash_empty_body),
                            style = MaterialTheme.typography.bodyMedium,
                            color = p.muted,
                            textAlign = TextAlign.Center,
                        )
                    }
                }
                else -> LazyVerticalGrid(
                    columns = GridCells.Fixed(4),
                    modifier = Modifier.fillMaxSize().testTag("trash_grid"),
                ) {
                    items(state.entries, key = { "t${it.item.id}" }, contentType = { "cell" }) { e ->
                        val days = daysLeft(e, now)
                        SelectablePhotoCell(
                            item = e.item,
                            selected = e.item.id in state.selection,
                            selecting = selecting,
                            // Trashed files are not in the timeline: a tap selects instead of opening the viewer.
                            onOpen = { vm.toggle(e.item.id) },
                            onToggle = { vm.toggle(e.item.id) },
                            label = pluralStringResource(R.plurals.trash_days_left, days, days),
                            shared = false,
                        )
                    }
                }
            }
        }
    }

    when (dialog) {
        TrashDialog.Restore -> AlbumPickerDialog(
            title = stringResource(R.string.trash_restore_to),
            loadTargets = vm::restoreTargets,
            noneLabel = stringResource(R.string.album_uncategorized),
            onPick = { id -> vm.restoreSelected(id); dialog = null },
            onDismiss = { dialog = null },
        )
        TrashDialog.Delete -> ConfirmDialog(
            title = pluralStringResource(R.plurals.trash_delete_title, state.selection.size, state.selection.size),
            body = stringResource(R.string.trash_delete_body),
            confirm = stringResource(R.string.trash_delete_forever),
            danger = true,
            onConfirm = { vm.deleteSelected(); dialog = null },
            onDismiss = { dialog = null },
        )
        TrashDialog.Empty -> ConfirmDialog(
            title = stringResource(R.string.trash_empty_confirm_title),
            body = pluralStringResource(R.plurals.trash_empty_confirm_body, state.entries.size, state.entries.size),
            confirm = stringResource(R.string.trash_empty_action),
            danger = true,
            onConfirm = { vm.emptyTrash(); dialog = null },
            onDismiss = { dialog = null },
        )
        null -> Unit
    }
}
