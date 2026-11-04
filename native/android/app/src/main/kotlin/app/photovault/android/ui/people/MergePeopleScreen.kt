@file:OptIn(ExperimentalMaterial3Api::class)

package app.photovault.android.ui.people

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.CircularProgressIndicator
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
import app.photovault.android.data.ml.Person
import app.photovault.android.ui.components.ProblemText
import app.photovault.android.ui.theme.Spacing
import app.photovault.android.ui.theme.palette

/** "Merge with…": pick the person this one is the same as, confirm, and the two become one. */
@Composable
fun MergePeopleScreen(vm: MergePeopleViewModel, onBack: () -> Unit, onDone: (personId: String) -> Unit) {
    val state by vm.state.collectAsStateWithLifecycle()
    val p = MaterialTheme.palette
    val done by rememberUpdatedState(onDone)
    LaunchedEffect(vm) { vm.doneFlow.collect { done(it.personId) } }
    var confirm by remember { mutableStateOf<Person?>(null) }
    val current = state.person

    Scaffold(
        containerColor = p.bg,
        topBar = {
            Column {
                TopAppBar(
                    colors = TopAppBarDefaults.topAppBarColors(containerColor = p.bg, titleContentColor = p.text),
                    navigationIcon = {
                        IconButton(onClick = onBack) {
                            Icon(painterResource(R.drawable.ic_arrow_back), contentDescription = stringResource(R.string.back), tint = p.text)
                        }
                    },
                    title = { Text(stringResource(R.string.merge_title, personLabel(current)), style = MaterialTheme.typography.titleLarge, maxLines = 1, overflow = TextOverflow.Ellipsis) },
                )
                if (state.busy) LinearProgressIndicator(Modifier.fillMaxWidth(), color = p.accent2, trackColor = p.panel)
            }
        },
    ) { padding ->
        if (!state.loaded) {
            Box(Modifier.fillMaxSize().padding(padding), contentAlignment = Alignment.Center) { CircularProgressIndicator(color = p.accent2) }
            return@Scaffold
        }
        LazyColumn(
            modifier = Modifier.fillMaxSize(),
            contentPadding = PaddingValues(
                start = Spacing.xl,
                end = Spacing.xl,
                top = padding.calculateTopPadding(),
                bottom = padding.calculateBottomPadding() + Spacing.section,
            ),
        ) {
            item(key = "intro") {
                Text(
                    stringResource(R.string.merge_intro),
                    style = MaterialTheme.typography.bodyMedium,
                    color = p.muted,
                    modifier = Modifier.padding(vertical = Spacing.md),
                )
            }
            state.error?.let { error -> item(key = "error") { ProblemText(error, Modifier.padding(vertical = Spacing.md)) } }
            if (state.candidates.isEmpty()) {
                item(key = "empty") {
                    Text(
                        stringResource(R.string.merge_none),
                        style = MaterialTheme.typography.bodyMedium,
                        color = p.muted,
                        textAlign = TextAlign.Center,
                        modifier = Modifier.fillMaxWidth().padding(Spacing.section),
                    )
                }
            }
            items(state.candidates, key = { it.id }) { other ->
                CandidateRow(other, enabled = !state.busy) { confirm = other }
            }
        }
    }

    val other = confirm
    if (other != null && current != null) {
        val (intoId, _) = MergePeopleViewModel.direction(current, other)
        val into = if (intoId == current.id) current else other
        val from = if (into === current) other else current
        AlertDialog(
            onDismissRequest = { confirm = null },
            title = { Text(stringResource(R.string.merge_confirm_title)) },
            text = { Text(stringResource(R.string.merge_confirm_body, personLabel(from), personLabel(into))) },
            confirmButton = {
                TextButton(onClick = {
                    confirm = null
                    vm.merge(other)
                }) { Text(stringResource(R.string.merge_action), color = p.accent2) }
            },
            dismissButton = {
                TextButton(onClick = { confirm = null }) { Text(stringResource(R.string.people_cancel), color = p.text) }
            },
            containerColor = p.panel,
        )
    }
}

@Composable
private fun CandidateRow(person: Person, enabled: Boolean, onClick: () -> Unit) {
    val p = MaterialTheme.palette
    Row(
        Modifier
            .fillMaxWidth()
            .clickable(enabled = enabled, onClick = onClick)
            .padding(vertical = Spacing.sm),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(Spacing.lg),
    ) {
        FaceAvatar(person.cover?.faceId, 48.dp)
        Column(Modifier.weight(1f)) {
            Text(
                personLabel(person),
                style = MaterialTheme.typography.titleMedium,
                color = if (person.name == null) p.muted else p.text,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
            Text(
                pluralStringResource(R.plurals.search_photos, person.fileCount.toInt(), person.fileCount.toInt()),
                style = MaterialTheme.typography.bodySmall,
                color = p.muted,
            )
        }
    }
}
