@file:OptIn(ExperimentalMaterial3Api::class)

package app.photovault.android.ui.people

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBar
import androidx.compose.material3.TopAppBarDefaults
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.res.pluralStringResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import app.photovault.android.R
import app.photovault.android.data.ml.Person
import app.photovault.android.ui.components.PrimaryButton
import app.photovault.android.ui.components.ProblemText
import app.photovault.android.ui.components.PvTextField
import app.photovault.android.ui.components.SectionLabel
import app.photovault.android.ui.theme.Spacing
import app.photovault.android.ui.theme.palette
import androidx.compose.ui.text.input.ImeAction

/** Name a cluster or rename a person; picking an existing name merges into that person. */
@Composable
fun NamePersonScreen(vm: NamePersonViewModel, onBack: () -> Unit, onDone: (personId: String) -> Unit) {
    val state by vm.state.collectAsStateWithLifecycle()
    val p = MaterialTheme.palette
    val done by rememberUpdatedState(onDone)
    LaunchedEffect(vm) { vm.doneFlow.collect { done(it.personId) } }
    val focus = remember { FocusRequester() }
    LaunchedEffect(state.loaded) { if (state.loaded) focus.requestFocus() }

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
                    Text(
                        stringResource(if (state.person?.name == null) R.string.name_title_new else R.string.name_title_rename),
                        style = MaterialTheme.typography.titleLarge,
                    )
                },
            )
        },
    ) { padding ->
        LazyColumn(
            modifier = Modifier.fillMaxSize(),
            contentPadding = PaddingValues(
                start = Spacing.xl,
                end = Spacing.xl,
                top = padding.calculateTopPadding() + Spacing.md,
                bottom = padding.calculateBottomPadding() + Spacing.section,
            ),
            verticalArrangement = Arrangement.spacedBy(Spacing.md),
        ) {
            item(key = "form") {
                Column(horizontalAlignment = Alignment.CenterHorizontally, verticalArrangement = Arrangement.spacedBy(Spacing.lg)) {
                    FaceAvatar(state.person?.cover?.faceId, 96.dp)
                    PvTextField(
                        value = state.name,
                        onValueChange = vm::onName,
                        label = stringResource(R.string.name_label),
                        modifier = Modifier.focusRequester(focus),
                        imeAction = ImeAction.Done,
                        onIme = vm::save,
                        enabled = state.loaded && !state.saving,
                    )
                    state.error?.let { ProblemText(it) }
                    PrimaryButton(
                        text = stringResource(R.string.name_save),
                        onClick = vm::save,
                        modifier = Modifier.fillMaxWidth(),
                        enabled = state.loaded && state.name.isNotBlank(),
                        loading = state.saving,
                    )
                }
            }
            if (state.matches.isNotEmpty()) {
                item(key = "matches-h") { SectionLabel(stringResource(R.string.name_same_as)) }
                items(state.matches, key = { it.id }) { other -> MatchRow(other, enabled = !state.saving) { vm.mergeInto(other) } }
            }
        }
    }
}

@Composable
private fun MatchRow(person: Person, enabled: Boolean, onClick: () -> Unit) {
    val p = MaterialTheme.palette
    Row(
        Modifier
            .fillMaxWidth()
            .clickable(enabled = enabled, onClick = onClick)
            .padding(vertical = Spacing.sm),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(Spacing.lg),
    ) {
        FaceAvatar(person.cover?.faceId, 40.dp)
        Column(Modifier.weight(1f)) {
            Text(personLabel(person), style = MaterialTheme.typography.titleMedium, color = p.text, maxLines = 1, overflow = TextOverflow.Ellipsis)
            Text(
                pluralStringResource(R.plurals.search_photos, person.fileCount.toInt(), person.fileCount.toInt()),
                style = MaterialTheme.typography.bodySmall,
                color = p.muted,
            )
        }
        Text(stringResource(R.string.name_merge_into), style = MaterialTheme.typography.labelLarge, color = p.accent2)
    }
}
