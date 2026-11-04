@file:OptIn(ExperimentalMaterial3Api::class)

package app.photovault.android.ui.people

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.grid.GridItemSpan
import androidx.compose.foundation.lazy.grid.rememberLazyGridState
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.material3.TopAppBarDefaults
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.res.pluralStringResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import app.photovault.android.R
import app.photovault.android.ui.components.LinkButton
import app.photovault.android.ui.components.PanelRow
import app.photovault.android.ui.components.ProblemText
import app.photovault.android.ui.search.ResultsGrid
import app.photovault.android.ui.search.ScrollToReturned
import app.photovault.android.ui.theme.Spacing
import app.photovault.android.ui.theme.palette
import kotlinx.coroutines.flow.Flow

/**
 * One person: cover face, name and photos, with the actions of Ente's person page: name / rename, merge,
 * review suggestions, remove wrong photos, use a photo as the cover, hide, remove the name.
 * Based on Ente (ente.com).
 */
@Composable
fun PersonScreen(
    vm: PersonViewModel,
    returnIndex: Flow<Int?>,
    onReturnConsumed: () -> Unit,
    onBack: () -> Unit,
    onOpen: (source: String, index: Int, fileId: Long) -> Unit,
    onName: (personId: String) -> Unit,
    onMerge: (personId: String) -> Unit,
    onSuggestions: (personId: String) -> Unit,
) {
    val state by vm.state.collectAsStateWithLifecycle()
    val p = MaterialTheme.palette
    val gridState = rememberLazyGridState()
    var confirmRemoveName by remember { mutableStateOf(false) }
    val back by rememberUpdatedState(onBack)
    LaunchedEffect(vm) {
        vm.eventFlow.collect { e ->
            when (e) {
                PersonEvent.Gone -> back()
            }
        }
    }
    // Back from the name / merge / suggestion screens: core's data changed.
    LaunchedEffect(vm) { vm.refresh() }

    val person = state.person
    val name = personLabel(person)
    val topItems = if (state.suggestionCount > 0 && !state.selecting) 2 else 1
    ScrollToReturned(returnIndex, onReturnConsumed, { vm.state.value.grid }, gridState, offset = topItems)

    Scaffold(
        containerColor = p.bg,
        topBar = {
            Column {
                if (state.selecting) {
                    SelectionBar(
                        count = state.selected.size,
                        onCancel = vm::cancelSelecting,
                        onRemove = vm::removeSelected,
                        onCover = {
                            state.selected.singleOrNull()?.let(vm::useAsCover)
                            vm.cancelSelecting()
                        },
                    )
                } else {
                    TopAppBar(
                        colors = TopAppBarDefaults.topAppBarColors(containerColor = p.bg, titleContentColor = p.text),
                        navigationIcon = {
                            IconButton(onClick = onBack) {
                                Icon(painterResource(R.drawable.ic_arrow_back), contentDescription = stringResource(R.string.back), tint = p.text)
                            }
                        },
                        title = { Text(name, style = MaterialTheme.typography.titleLarge, maxLines = 1, overflow = TextOverflow.Ellipsis) },
                        actions = {
                            if (person != null) {
                                PersonMenu(
                                    named = person.name != null,
                                    hidden = person.isHidden,
                                    hasSuggestions = state.suggestionCount > 0,
                                    onName = { onName(state.personId) },
                                    onMerge = { onMerge(state.personId) },
                                    onSuggestions = { onSuggestions(state.personId) },
                                    onSelect = vm::startSelecting,
                                    onHide = { vm.setHidden(!person.isHidden) },
                                    onRemoveName = { confirmRemoveName = true },
                                )
                            }
                        },
                    )
                }
                if (state.busy) LinearProgressIndicator(Modifier.fillMaxWidth(), color = p.accent2, trackColor = p.panel)
            }
        },
    ) { padding ->
        when {
            !state.loaded && person == null -> Box(Modifier.fillMaxSize().padding(padding), contentAlignment = Alignment.Center) {
                CircularProgressIndicator(color = p.accent2)
            }
            person == null -> Box(Modifier.fillMaxSize().padding(padding).padding(Spacing.section), contentAlignment = Alignment.Center) {
                Text(
                    state.error ?: stringResource(R.string.person_gone),
                    style = MaterialTheme.typography.bodyMedium,
                    color = p.muted,
                    textAlign = TextAlign.Center,
                )
            }
            else -> ResultsGrid(
                model = state.grid,
                state = gridState,
                contentPadding = padding,
                onOpen = { index, fileId ->
                    vm.prepareViewer()
                    onOpen(vm.source, index, fileId)
                },
                selected = if (state.selecting) state.selected else null,
                onToggle = vm::toggle,
            ) {
                item(key = "person-header", span = { GridItemSpan(maxLineSpan) }, contentType = "person-header") {
                    PersonHeader(
                        faceId = person.cover?.faceId,
                        name = name,
                        named = person.name != null,
                        hidden = person.isHidden,
                        fileCount = person.fileCount.toInt(),
                        selecting = state.selecting,
                        error = state.error,
                        onName = { onName(state.personId) },
                    )
                }
                if (state.suggestionCount > 0 && !state.selecting) {
                    item(key = "person-suggestions", span = { GridItemSpan(maxLineSpan) }, contentType = "person-suggestions") {
                        PanelRow(
                            title = stringResource(R.string.person_review),
                            subtitle = pluralStringResource(R.plurals.person_review_count, state.suggestionCount, state.suggestionCount),
                            onClick = { onSuggestions(state.personId) },
                            icon = R.drawable.ic_people,
                            modifier = Modifier.padding(horizontal = Spacing.lg, vertical = Spacing.md),
                        )
                    }
                }
            }
        }
    }

    if (confirmRemoveName) {
        AlertDialog(
            onDismissRequest = { confirmRemoveName = false },
            title = { Text(stringResource(R.string.person_remove_name_title, name)) },
            text = { Text(stringResource(R.string.person_remove_name_body)) },
            confirmButton = {
                TextButton(onClick = {
                    confirmRemoveName = false
                    vm.removeName()
                }) { Text(stringResource(R.string.person_remove_name), color = p.err) }
            },
            dismissButton = {
                TextButton(onClick = { confirmRemoveName = false }) { Text(stringResource(R.string.people_cancel), color = p.text) }
            },
            containerColor = p.panel,
        )
    }
}

@Composable
private fun PersonHeader(
    faceId: String?,
    name: String,
    named: Boolean,
    hidden: Boolean,
    fileCount: Int,
    selecting: Boolean,
    error: String?,
    onName: () -> Unit,
) {
    val p = MaterialTheme.palette
    Column(
        Modifier.fillMaxWidth().padding(horizontal = Spacing.xl, vertical = Spacing.lg),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.spacedBy(Spacing.sm),
    ) {
        FaceAvatar(faceId, 96.dp)
        if (named) {
            Text(name, style = MaterialTheme.typography.headlineSmall, color = p.text, maxLines = 2, textAlign = TextAlign.Center)
        } else {
            LinkButton(stringResource(R.string.person_add_name), onClick = onName)
        }
        Text(pluralStringResource(R.plurals.search_photos, fileCount, fileCount), style = MaterialTheme.typography.bodySmall, color = p.muted)
        if (hidden) Text(stringResource(R.string.person_hidden_note), style = MaterialTheme.typography.bodySmall, color = p.muted, textAlign = TextAlign.Center)
        if (selecting) {
            Text(stringResource(R.string.person_select_hint), style = MaterialTheme.typography.bodyMedium, color = p.text, textAlign = TextAlign.Center)
        }
        if (error != null) ProblemText(error)
    }
}

@Composable
private fun PersonMenu(
    named: Boolean,
    hidden: Boolean,
    hasSuggestions: Boolean,
    onName: () -> Unit,
    onMerge: () -> Unit,
    onSuggestions: () -> Unit,
    onSelect: () -> Unit,
    onHide: () -> Unit,
    onRemoveName: () -> Unit,
) {
    val p = MaterialTheme.palette
    var open by remember { mutableStateOf(false) }
    IconButton(onClick = { open = true }) {
        Icon(painterResource(R.drawable.ic_more_vert), contentDescription = stringResource(R.string.people_more), tint = p.text)
    }
    DropdownMenu(expanded = open, onDismissRequest = { open = false }) {
        MenuItem(if (named) R.string.person_rename else R.string.person_add_name, R.drawable.ic_edit) { open = false; onName() }
        MenuItem(R.string.person_merge, R.drawable.ic_merge) { open = false; onMerge() }
        if (hasSuggestions) MenuItem(R.string.person_review, R.drawable.ic_people) { open = false; onSuggestions() }
        MenuItem(R.string.person_remove_photos, R.drawable.ic_delete) { open = false; onSelect() }
        MenuItem(if (hidden) R.string.person_unhide else R.string.person_hide, if (hidden) R.drawable.ic_visibility else R.drawable.ic_visibility_off) {
            open = false
            onHide()
        }
        if (named) MenuItem(R.string.person_remove_name, R.drawable.ic_close) { open = false; onRemoveName() }
    }
}

@Composable
private fun MenuItem(text: Int, icon: Int, onClick: () -> Unit) {
    DropdownMenuItem(
        text = { Text(stringResource(text)) },
        leadingIcon = { Icon(painterResource(icon), contentDescription = null) },
        onClick = onClick,
    )
}

@Composable
private fun SelectionBar(count: Int, onCancel: () -> Unit, onRemove: () -> Unit, onCover: () -> Unit) {
    val p = MaterialTheme.palette
    TopAppBar(
        colors = TopAppBarDefaults.topAppBarColors(containerColor = p.panel, titleContentColor = p.text),
        navigationIcon = {
            IconButton(onClick = onCancel) {
                Icon(painterResource(R.drawable.ic_close), contentDescription = stringResource(R.string.people_cancel), tint = p.text)
            }
        },
        title = { Text(pluralStringResource(R.plurals.person_selected, count, count), style = MaterialTheme.typography.titleMedium) },
        actions = {
            IconButton(onClick = onCover, enabled = count == 1) {
                Icon(
                    painterResource(R.drawable.ic_account),
                    contentDescription = stringResource(R.string.person_use_as_cover),
                    tint = if (count == 1) p.text else p.faint,
                )
            }
            TextButton(onClick = onRemove, enabled = count > 0) {
                Text(stringResource(R.string.person_remove_selected), color = if (count > 0) p.err else p.faint)
            }
        },
    )
}
