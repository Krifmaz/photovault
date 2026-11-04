@file:OptIn(ExperimentalMaterial3Api::class)

package app.photovault.android.ui.albums

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.grid.GridCells
import androidx.compose.foundation.lazy.grid.LazyVerticalGrid
import androidx.compose.foundation.lazy.grid.itemsIndexed
import androidx.compose.foundation.lazy.grid.rememberLazyGridState
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
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.runtime.snapshotFlow
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.res.pluralStringResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import app.photovault.android.R
import app.photovault.android.data.albums.AlbumDetails
import app.photovault.android.ui.theme.Spacing
import app.photovault.android.ui.theme.palette
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.distinctUntilChanged

private enum class AlbumDialog { Rename, Delete, Leave, Trash, AddTo, UnhideTo }

/** One album (or the hidden-photos collection): grid, selection and the album's actions. */
@Composable
fun AlbumScreen(
    vm: AlbumViewModel,
    returnIndex: Flow<Int?>,
    onReturnConsumed: () -> Unit,
    onOpen: (index: Int, fileId: Long) -> Unit,
    onShare: () -> Unit,
    onBack: () -> Unit,
) {
    val state by vm.state.collectAsStateWithLifecycle()
    val items by vm.items.collectAsStateWithLifecycle()
    val p = MaterialTheme.palette
    val context = LocalContext.current
    val snackbar = remember { SnackbarHostState() }
    val gridState = rememberLazyGridState()
    var dialog by rememberSaveable { mutableStateOf<AlbumDialog?>(null) }
    var menu by remember { mutableStateOf(false) }
    val back by rememberUpdatedState(onBack)
    val details = state.details
    val selecting = state.selection.isNotEmpty()

    BackHandler(enabled = selecting) { vm.clearSelection() }

    LaunchedEffect(vm) {
        vm.eventFlow.collect { e ->
            when (e) {
                AlbumEvent.Gone -> back()
                is AlbumEvent.Done -> {
                    val res = when (e.kind) {
                        DoneKind.Added -> R.plurals.album_done_added
                        DoneKind.Removed -> R.plurals.album_done_removed
                        DoneKind.Trashed -> R.plurals.album_done_trashed
                        DoneKind.Hidden -> R.plurals.album_done_hidden
                        DoneKind.Unhidden -> R.plurals.album_done_unhidden
                    }
                    snackbar.showSnackbar(context.resources.getQuantityString(res, e.count, e.count))
                }
            }
        }
    }
    LaunchedEffect(state.error) {
        state.error?.let {
            snackbar.showSnackbar(it)
            vm.clearError()
        }
    }
    // Back from the viewer: keep the photo it ended on in view.
    LaunchedEffect(returnIndex) {
        returnIndex.collect { index ->
            if (index == null) return@collect
            onReturnConsumed()
            val visible = gridState.layoutInfo.visibleItemsInfo
            val first = visible.firstOrNull()?.index ?: 0
            val last = visible.lastOrNull()?.index ?: 0
            if (index !in (first + 1) until last) gridState.scrollToItem(index)
        }
    }
    LaunchedEffect(gridState, vm) {
        snapshotFlow { gridState.layoutInfo.visibleItemsInfo.lastOrNull()?.index ?: -1 }
            .distinctUntilChanged()
            .collect { if (it >= 0) vm.onVisible(it) }
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
                    ) { SelectionActions(details, onDialog = { dialog = it }, vm = vm) }
                } else {
                    val count = details?.collection?.count?.toInt() ?: 0
                    val subtitle = when {
                        details == null -> null
                        !details.owned -> stringResource(R.string.album_count_by, pluralStringResource(R.plurals.album_items, count, count), details.ownerEmail)
                        else -> pluralStringResource(R.plurals.album_items, count, count)
                    }
                    AlbumTopBar(
                        title = details?.let { albumName(it) }.orEmpty(),
                        subtitle = subtitle,
                        onBack = onBack,
                    ) {
                        if (details != null && details.owned && !details.isDefaultHidden) {
                            IconButton(onClick = onShare, modifier = Modifier.testTag("album_share")) {
                                Icon(painterResource(R.drawable.ic_share), contentDescription = stringResource(R.string.album_share), tint = p.text)
                            }
                        }
                        if (details != null && !details.isDefaultHidden) {
                            IconButton(onClick = { menu = true }) {
                                Icon(painterResource(R.drawable.ic_more_vert), contentDescription = stringResource(R.string.albums_more), tint = p.text)
                            }
                            AlbumMenu(details, menu, onDismiss = { menu = false }, onShare = onShare, onDialog = { dialog = it }, vm = vm)
                        }
                    }
                }
                if (state.busy) LinearProgressIndicator(Modifier.padding(0.dp), color = p.accent2, trackColor = p.panel)
            }
        },
    ) { padding ->
        Box(Modifier.fillMaxSize().padding(padding)) {
            when {
                !state.loaded -> CircularProgressIndicator(color = p.accent2, modifier = Modifier.align(Alignment.Center))
                details == null -> Text(
                    stringResource(R.string.album_missing),
                    color = p.muted,
                    modifier = Modifier.align(Alignment.Center).padding(Spacing.section),
                )
                items.isEmpty() -> Column(
                    Modifier.align(Alignment.Center).padding(Spacing.section),
                    horizontalAlignment = Alignment.CenterHorizontally,
                    verticalArrangement = Arrangement.spacedBy(Spacing.md),
                ) {
                    Text(stringResource(R.string.album_empty_title), style = MaterialTheme.typography.titleLarge, color = p.text)
                    Text(stringResource(R.string.album_empty_body), style = MaterialTheme.typography.bodyMedium, color = p.muted, textAlign = TextAlign.Center)
                }
                else -> LazyVerticalGrid(
                    columns = GridCells.Fixed(4),
                    state = gridState,
                    modifier = Modifier.fillMaxSize().testTag("album_photo_grid"),
                ) {
                    itemsIndexed(items, key = { _, item -> "p${item.id}" }, contentType = { _, _ -> "cell" }) { index, item ->
                        SelectablePhotoCell(
                            item = item,
                            selected = item.id in state.selection,
                            selecting = selecting,
                            onOpen = { onOpen(index, item.id) },
                            onToggle = { vm.toggle(item.id) },
                        )
                    }
                }
            }
        }
    }

    val d = details
    when (dialog) {
        AlbumDialog.Rename -> if (d != null) NameDialog(
            title = stringResource(R.string.album_rename),
            initial = d.collection.name,
            confirm = stringResource(R.string.albums_save),
            onConfirm = { vm.rename(it); dialog = null },
            onDismiss = { dialog = null },
        )
        AlbumDialog.Delete -> DeleteAlbumDialog(
            onKeep = { vm.delete(keepFiles = true); dialog = null },
            onTrash = { vm.delete(keepFiles = false); dialog = null },
            onDismiss = { dialog = null },
        )
        AlbumDialog.Leave -> ConfirmDialog(
            title = stringResource(R.string.album_leave_title),
            body = stringResource(R.string.album_leave_body),
            confirm = stringResource(R.string.album_leave),
            danger = true,
            onConfirm = { vm.leave(); dialog = null },
            onDismiss = { dialog = null },
        )
        AlbumDialog.Trash -> ConfirmDialog(
            title = pluralStringResource(R.plurals.album_trash_title, state.selection.size, state.selection.size),
            body = stringResource(R.string.album_trash_body),
            confirm = stringResource(R.string.album_move_to_trash),
            danger = true,
            onConfirm = { vm.trashSelected(); dialog = null },
            onDismiss = { dialog = null },
        )
        AlbumDialog.AddTo -> AlbumPickerDialog(
            title = stringResource(R.string.album_add_to),
            loadTargets = vm::targets,
            onPick = { id -> if (id != null) vm.addSelectedTo(id); dialog = null },
            allowNew = true,
            onNew = { vm.addSelectedToNew(it); dialog = null },
            onDismiss = { dialog = null },
        )
        AlbumDialog.UnhideTo -> AlbumPickerDialog(
            title = stringResource(R.string.album_unhide_to),
            loadTargets = vm::targets,
            noneLabel = stringResource(R.string.album_uncategorized),
            onPick = { id -> vm.unhideSelected(id); dialog = null },
            onDismiss = { dialog = null },
        )
        null -> Unit
    }
}

@Composable
private fun AlbumMenu(
    d: AlbumDetails,
    expanded: Boolean,
    onDismiss: () -> Unit,
    onShare: () -> Unit,
    onDialog: (AlbumDialog) -> Unit,
    vm: AlbumViewModel,
) {
    DropdownMenu(expanded = expanded, onDismissRequest = onDismiss) {
        if (d.owned) {
            DropdownMenuItem(text = { Text(stringResource(R.string.album_rename)) }, onClick = { onDismiss(); onDialog(AlbumDialog.Rename) })
            DropdownMenuItem(text = { Text(stringResource(R.string.album_share)) }, onClick = { onDismiss(); onShare() })
            DropdownMenuItem(
                text = { Text(stringResource(if (d.hidden) R.string.album_unhide else R.string.album_hide)) },
                onClick = { onDismiss(); vm.setHidden(!d.hidden) },
            )
            DropdownMenuItem(text = { Text(stringResource(R.string.album_delete)) }, onClick = { onDismiss(); onDialog(AlbumDialog.Delete) })
        } else {
            DropdownMenuItem(text = { Text(stringResource(R.string.album_leave)) }, onClick = { onDismiss(); onDialog(AlbumDialog.Leave) })
        }
    }
}

@Composable
private fun SelectionActions(d: AlbumDetails?, onDialog: (AlbumDialog) -> Unit, vm: AlbumViewModel) {
    val p = MaterialTheme.palette
    var more by remember { mutableStateOf(false) }
    if (d == null) return
    if (d.isDefaultHidden) {
        IconButton(onClick = { onDialog(AlbumDialog.UnhideTo) }) {
            Icon(painterResource(R.drawable.ic_visibility), contentDescription = stringResource(R.string.album_unhide_photos), tint = p.text)
        }
    } else {
        IconButton(onClick = { onDialog(AlbumDialog.AddTo) }) {
            Icon(painterResource(R.drawable.ic_add), contentDescription = stringResource(R.string.album_add_to), tint = p.text)
        }
    }
    if (d.owned) {
        IconButton(onClick = { onDialog(AlbumDialog.Trash) }) {
            Icon(painterResource(R.drawable.ic_delete), contentDescription = stringResource(R.string.album_move_to_trash), tint = p.text)
        }
    }
    IconButton(onClick = { more = true }) {
        Icon(painterResource(R.drawable.ic_more_vert), contentDescription = stringResource(R.string.albums_more), tint = p.text)
    }
    DropdownMenu(expanded = more, onDismissRequest = { more = false }) {
        DropdownMenuItem(text = { Text(stringResource(R.string.albums_select_all)) }, onClick = { more = false; vm.selectAll() })
        if (d.owned && !d.isDefaultHidden) {
            DropdownMenuItem(text = { Text(stringResource(R.string.album_remove_from)) }, onClick = { more = false; vm.removeSelected() })
            DropdownMenuItem(text = { Text(stringResource(R.string.album_hide_photos)) }, onClick = { more = false; vm.hideSelected() })
        } else if (!d.owned && d.canAdd) {
            DropdownMenuItem(text = { Text(stringResource(R.string.album_remove_from)) }, onClick = { more = false; vm.removeSelected() })
        }
    }
}

@Composable
private fun DeleteAlbumDialog(onKeep: () -> Unit, onTrash: () -> Unit, onDismiss: () -> Unit) {
    val p = MaterialTheme.palette
    androidx.compose.material3.AlertDialog(
        onDismissRequest = onDismiss,
        containerColor = p.bg2,
        title = { Text(stringResource(R.string.album_delete_title), color = p.text) },
        text = { Text(stringResource(R.string.album_delete_body), color = p.muted) },
        confirmButton = {
            Column(horizontalAlignment = Alignment.End) {
                androidx.compose.material3.TextButton(onClick = onKeep) { Text(stringResource(R.string.album_delete_keep), color = p.accent2) }
                androidx.compose.material3.TextButton(onClick = onTrash) { Text(stringResource(R.string.album_delete_trash), color = p.err) }
                androidx.compose.material3.TextButton(onClick = onDismiss) { Text(stringResource(R.string.albums_cancel), color = p.muted) }
            }
        },
    )
}
