package app.photovault.android.ui.search

import androidx.annotation.DrawableRes
import androidx.annotation.StringRes
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.LazyRow
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.OutlinedTextFieldDefaults
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.platform.LocalSoftwareKeyboardController
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.res.pluralStringResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import app.photovault.android.R
import app.photovault.android.data.ml.SearchKind
import app.photovault.android.data.ml.SearchSuggestion
import app.photovault.android.ui.components.LinkButton
import app.photovault.android.ui.components.PanelRow
import app.photovault.android.ui.components.ProblemText
import app.photovault.android.ui.components.SectionLabel
import app.photovault.android.ui.people.FaceAvatar
import app.photovault.android.ui.people.Pill
import app.photovault.android.ui.people.SmallThumb
import app.photovault.android.ui.theme.EncoreRadius
import app.photovault.android.ui.theme.Spacing
import app.photovault.android.ui.theme.palette

/**
 * Search: a query field (suggestions update as you type, debounced in [SearchViewModel]) and, before typing,
 * the discovery content (people, places, file types, albums, magic prompts). Self-contained so it can be a
 * route of its own or a tab: [onBack] null hides the back arrow. The sections follow Ente's search screen.
 * Based on Ente (ente.com).
 */
@Composable
fun SearchScreen(
    vm: SearchViewModel,
    onBack: (() -> Unit)?,
    onOpen: (SearchSuggestion) -> Unit,
    onPeople: () -> Unit,
) {
    val state by vm.state.collectAsStateWithLifecycle()
    val p = MaterialTheme.palette
    val keyboard = LocalSoftwareKeyboardController.current
    val open: (SearchSuggestion) -> Unit = { s ->
        keyboard?.hide()
        onOpen(s)
    }
    // People may have been named or merged while away (person screens): reload the discovery row.
    LaunchedEffect(vm) { vm.refreshDiscovery() }

    Scaffold(
        containerColor = p.bg,
        topBar = {
            SearchField(
                query = state.query,
                onQuery = vm::onQuery,
                onClear = vm::clear,
                onSubmit = { vm.submitTarget()?.let(open) },
                onBack = onBack,
            )
        },
    ) { padding ->
        Box(Modifier.fillMaxSize().padding(padding)) {
            if (state.query.isBlank()) {
                DiscoveryContent(state, onOpen = open, onPeople = onPeople, onEnableMl = vm::enableMl)
            } else {
                SuggestionList(state, onOpen = open)
            }
            if (state.searching) {
                LinearProgressIndicator(Modifier.fillMaxWidth().align(Alignment.TopCenter), color = p.accent2, trackColor = p.panel)
            }
        }
    }
}

@Composable
private fun SearchField(
    query: String,
    onQuery: (String) -> Unit,
    onClear: () -> Unit,
    onSubmit: () -> Unit,
    onBack: (() -> Unit)?,
) {
    val p = MaterialTheme.palette
    val focus = remember { FocusRequester() }
    // Focus (and the keyboard) only on first open, not again when coming back from results.
    var focusedOnce by rememberSaveable { mutableStateOf(false) }
    LaunchedEffect(Unit) {
        if (!focusedOnce) {
            focusedOnce = true
            focus.requestFocus()
        }
    }
    Row(
        Modifier
            .fillMaxWidth()
            .statusBarsPadding()
            .padding(horizontal = Spacing.xs, vertical = Spacing.sm),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        if (onBack != null) {
            IconButton(onClick = onBack) {
                Icon(painterResource(R.drawable.ic_arrow_back), contentDescription = stringResource(R.string.back), tint = p.text)
            }
        } else {
            Box(Modifier.width(Spacing.md))
        }
        OutlinedTextField(
            value = query,
            onValueChange = onQuery,
            modifier = Modifier
                .weight(1f)
                .padding(end = Spacing.md)
                .focusRequester(focus)
                .testTag("search_field"),
            placeholder = { Text(stringResource(R.string.search_hint), color = p.faint) },
            leadingIcon = { Icon(painterResource(R.drawable.ic_search), contentDescription = null, tint = p.muted) },
            trailingIcon = if (query.isEmpty()) null else {
                {
                    IconButton(onClick = onClear) {
                        Icon(painterResource(R.drawable.ic_close), contentDescription = stringResource(R.string.search_clear), tint = p.muted)
                    }
                }
            },
            singleLine = true,
            shape = RoundedCornerShape(EncoreRadius.pill),
            keyboardOptions = KeyboardOptions(imeAction = ImeAction.Search),
            keyboardActions = KeyboardActions(onSearch = { onSubmit() }),
            colors = OutlinedTextFieldDefaults.colors(
                focusedContainerColor = p.panel,
                unfocusedContainerColor = p.panel,
                focusedBorderColor = p.accent2,
                unfocusedBorderColor = p.border,
                cursorColor = p.accent2,
                focusedTextColor = p.text,
                unfocusedTextColor = p.text,
            ),
        )
    }
}

// ---- before typing ----

@Composable
private fun DiscoveryContent(
    state: SearchState,
    onOpen: (SearchSuggestion) -> Unit,
    onPeople: () -> Unit,
    onEnableMl: () -> Unit,
) {
    val p = MaterialTheme.palette
    val d = state.discovery
    if (!state.discoveryLoaded) {
        Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) { CircularProgressIndicator(color = p.accent2) }
        return
    }
    LazyColumn(
        modifier = Modifier.fillMaxSize().testTag("search_discovery"),
        contentPadding = PaddingValues(bottom = Spacing.section),
    ) {
        state.error?.let { error ->
            item(key = "error") { ProblemText(error, Modifier.padding(horizontal = Spacing.xl, vertical = Spacing.md)) }
        }
        if (!state.mlEnabled) {
            item(key = "ml") {
                PanelRow(
                    title = stringResource(R.string.search_ml_title),
                    subtitle = stringResource(R.string.search_ml_body),
                    onClick = onEnableMl,
                    icon = R.drawable.ic_magic,
                    modifier = Modifier.padding(horizontal = Spacing.xl, vertical = Spacing.md),
                    trailing = {
                        if (state.mlBusy) CircularProgressIndicator(Modifier.size(20.dp), color = p.accent2, strokeWidth = 2.dp)
                        else Text(stringResource(R.string.search_ml_enable), color = p.accent2, style = MaterialTheme.typography.labelLarge)
                    },
                )
            }
        }
        if (d.people.isNotEmpty()) {
            item(key = "people-h") {
                Row(Modifier.fillMaxWidth().padding(start = Spacing.xl, end = Spacing.xs), verticalAlignment = Alignment.CenterVertically) {
                    SectionLabel(stringResource(R.string.search_section_people), Modifier.weight(1f))
                    LinkButton(stringResource(R.string.search_see_all), onClick = onPeople, modifier = Modifier.padding(top = Spacing.md))
                }
            }
            item(key = "people") {
                LazyRow(
                    contentPadding = PaddingValues(horizontal = Spacing.xl),
                    horizontalArrangement = Arrangement.spacedBy(Spacing.lg),
                ) {
                    items(d.people, key = { it.key }) { person -> PersonBubble(person, onOpen) }
                }
            }
        }
        pillRow("places", R.string.search_section_places, d.places, R.drawable.ic_place, onOpen)
        pillRow("types", R.string.search_section_types, d.fileTypes, R.drawable.ic_image, onOpen)
        pillRow("dates", R.string.search_section_dates, d.dates, R.drawable.ic_calendar, onOpen)
        if (d.albums.isNotEmpty()) {
            item(key = "albums-h") { SectionLabel(stringResource(R.string.search_section_albums), Modifier.padding(horizontal = Spacing.xl)) }
            item(key = "albums") {
                LazyRow(
                    contentPadding = PaddingValues(horizontal = Spacing.xl),
                    horizontalArrangement = Arrangement.spacedBy(Spacing.lg),
                ) {
                    items(d.albums, key = { it.key }) { album -> AlbumTile(album, onOpen) }
                }
            }
        }
        pillRow("magic", R.string.search_section_magic, d.magic, R.drawable.ic_magic, onOpen)
        if (d.isEmpty && state.mlEnabled) {
            item(key = "empty") {
                Text(
                    stringResource(R.string.search_discovery_empty),
                    style = MaterialTheme.typography.bodyMedium,
                    color = p.muted,
                    textAlign = TextAlign.Center,
                    modifier = Modifier.fillMaxWidth().padding(Spacing.section),
                )
            }
        }
    }
}

private fun androidx.compose.foundation.lazy.LazyListScope.pillRow(
    key: String,
    @StringRes title: Int,
    list: List<SearchSuggestion>,
    @DrawableRes icon: Int,
    onOpen: (SearchSuggestion) -> Unit,
) {
    if (list.isEmpty()) return
    item(key = "$key-h") { SectionLabel(stringResource(title), Modifier.padding(horizontal = Spacing.xl)) }
    item(key = key) {
        LazyRow(
            contentPadding = PaddingValues(horizontal = Spacing.xl),
            horizontalArrangement = Arrangement.spacedBy(Spacing.md),
        ) {
            items(list, key = { it.key }) { s -> Pill(s.label, onClick = { onOpen(s) }, icon = icon) }
        }
    }
}

@Composable
private fun PersonBubble(s: SearchSuggestion, onOpen: (SearchSuggestion) -> Unit) {
    val p = MaterialTheme.palette
    Column(
        Modifier
            .width(72.dp)
            .clickable { onOpen(s) }
            .padding(vertical = Spacing.xs),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.spacedBy(Spacing.xs),
    ) {
        FaceAvatar(s.coverFaceId, 64.dp)
        Text(
            s.label.ifEmpty { stringResource(R.string.people_unnamed_short) },
            style = MaterialTheme.typography.bodySmall,
            color = if (s.label.isEmpty()) p.muted else p.text,
            maxLines = 1,
            overflow = TextOverflow.Ellipsis,
        )
    }
}

@Composable
private fun AlbumTile(s: SearchSuggestion, onOpen: (SearchSuggestion) -> Unit) {
    val p = MaterialTheme.palette
    Column(Modifier.width(104.dp).clickable { onOpen(s) }, verticalArrangement = Arrangement.spacedBy(Spacing.xs)) {
        SmallThumb(s.coverFileId, 104.dp, fallback = R.drawable.ic_album)
        Text(s.label, style = MaterialTheme.typography.bodySmall, color = p.text, maxLines = 1, overflow = TextOverflow.Ellipsis)
        Text(
            pluralStringResource(R.plurals.search_photos, s.count.toInt(), s.count.toInt()),
            style = MaterialTheme.typography.labelSmall,
            color = p.muted,
            maxLines = 1,
        )
    }
}

// ---- while typing ----

@Composable
private fun SuggestionList(state: SearchState, onOpen: (SearchSuggestion) -> Unit) {
    val p = MaterialTheme.palette
    if (state.noResults) {
        Text(
            stringResource(R.string.search_no_results, state.query.trim()),
            style = MaterialTheme.typography.bodyMedium,
            color = p.muted,
            textAlign = TextAlign.Center,
            modifier = Modifier.fillMaxWidth().padding(Spacing.section),
        )
        return
    }
    LazyColumn(
        modifier = Modifier.fillMaxSize().testTag("search_suggestions"),
        contentPadding = PaddingValues(bottom = Spacing.section),
    ) {
        for (group in state.groups) {
            item(key = "h-${group.kind}", contentType = "header") {
                SectionLabel(stringResource(kindLabel(group.kind)), Modifier.padding(horizontal = Spacing.xl))
            }
            items(group.items, key = { "${it.kind}:${it.key}" }, contentType = { "row" }) { s -> SuggestionRow(s, onOpen) }
        }
    }
}

@Composable
private fun SuggestionRow(s: SearchSuggestion, onOpen: (SearchSuggestion) -> Unit) {
    val p = MaterialTheme.palette
    Row(
        Modifier
            .fillMaxWidth()
            .clickable { onOpen(s) }
            .padding(horizontal = Spacing.xl, vertical = Spacing.md),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(Spacing.lg),
    ) {
        when {
            s.kind == SearchKind.Person -> FaceAvatar(s.coverFaceId, 40.dp)
            s.coverFileId != null -> SmallThumb(s.coverFileId, 40.dp)
            else -> SmallThumb(null, 40.dp, fallback = kindIcon(s.kind))
        }
        Column(Modifier.weight(1f)) {
            Text(
                s.label.ifEmpty { stringResource(R.string.people_unnamed) },
                style = MaterialTheme.typography.titleMedium,
                color = p.text,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
            val sub = if (s.kind == SearchKind.Magic) stringResource(R.string.search_magic_row)
            else pluralStringResource(R.plurals.search_photos, s.count.toInt(), s.count.toInt())
            Text(sub, style = MaterialTheme.typography.bodySmall, color = p.muted, maxLines = 1)
        }
        Icon(painterResource(R.drawable.ic_chevron_right), contentDescription = null, tint = p.faint, modifier = Modifier.size(20.dp))
    }
}

private fun kindLabel(kind: SearchKind): Int = when (kind) {
    SearchKind.Magic -> R.string.search_section_magic
    SearchKind.Person -> R.string.search_section_people
    SearchKind.Album -> R.string.search_section_albums
    SearchKind.Date -> R.string.search_section_dates
    SearchKind.Place -> R.string.search_section_places
    SearchKind.FileName -> R.string.search_section_file_names
    SearchKind.Caption -> R.string.search_section_captions
    SearchKind.FileType -> R.string.search_section_types
}

@DrawableRes
internal fun kindIcon(kind: SearchKind): Int = when (kind) {
    SearchKind.Magic -> R.drawable.ic_magic
    SearchKind.Person -> R.drawable.ic_account
    SearchKind.Album -> R.drawable.ic_album
    SearchKind.Date -> R.drawable.ic_calendar
    SearchKind.Place -> R.drawable.ic_place
    SearchKind.FileName -> R.drawable.ic_text
    SearchKind.Caption -> R.drawable.ic_text
    SearchKind.FileType -> R.drawable.ic_image
}
