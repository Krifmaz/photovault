@file:OptIn(ExperimentalMaterial3Api::class)

package app.photovault.android.ui.people

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.grid.GridCells
import androidx.compose.foundation.lazy.grid.LazyVerticalGrid
import androidx.compose.foundation.lazy.grid.items
import androidx.compose.material3.Checkbox
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
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
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.res.pluralStringResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import app.photovault.android.R
import app.photovault.android.data.ml.Person
import app.photovault.android.ui.theme.Spacing
import app.photovault.android.ui.theme.palette

/** Everyone: round face avatars, named people first. */
@Composable
fun PeopleScreen(vm: PeopleViewModel, onBack: () -> Unit, onPerson: (String) -> Unit) {
    val state by vm.state.collectAsStateWithLifecycle()
    val p = MaterialTheme.palette
    var menu by remember { mutableStateOf(false) }
    // Coming back from a person (named, merged, hidden): reload.
    LaunchedEffect(vm) { vm.refresh() }

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
                title = { Text(stringResource(R.string.people_title), style = MaterialTheme.typography.titleLarge) },
                actions = {
                    IconButton(onClick = { menu = true }) {
                        Icon(painterResource(R.drawable.ic_more_vert), contentDescription = stringResource(R.string.people_more), tint = p.text)
                    }
                    DropdownMenu(expanded = menu, onDismissRequest = { menu = false }) {
                        DropdownMenuItem(
                            text = { Text(stringResource(R.string.people_show_hidden)) },
                            trailingIcon = { Checkbox(checked = state.showHidden, onCheckedChange = null) },
                            onClick = {
                                menu = false
                                vm.setShowHidden(!state.showHidden)
                            },
                        )
                    }
                },
            )
        },
    ) { padding ->
        when {
            !state.loaded -> Box(Modifier.fillMaxSize().padding(padding), contentAlignment = Alignment.Center) {
                CircularProgressIndicator(color = p.accent2)
            }
            state.people.isEmpty() -> Box(Modifier.fillMaxSize().padding(padding).padding(Spacing.section), contentAlignment = Alignment.Center) {
                Text(
                    state.error ?: stringResource(if (state.mlEnabled) R.string.people_empty else R.string.people_ml_off),
                    style = MaterialTheme.typography.bodyMedium,
                    color = p.muted,
                    textAlign = TextAlign.Center,
                )
            }
            else -> LazyVerticalGrid(
                columns = GridCells.Adaptive(minSize = 104.dp),
                contentPadding = PaddingValues(
                    start = Spacing.lg,
                    end = Spacing.lg,
                    top = padding.calculateTopPadding() + Spacing.md,
                    bottom = padding.calculateBottomPadding() + Spacing.section,
                ),
                verticalArrangement = Arrangement.spacedBy(Spacing.lg),
                modifier = Modifier.fillMaxSize().testTag("people_grid"),
            ) {
                items(state.people, key = { it.id }) { person -> PersonTile(person, onPerson) }
            }
        }
    }
}

@Composable
private fun PersonTile(person: Person, onPerson: (String) -> Unit) {
    val p = MaterialTheme.palette
    Column(
        Modifier
            .fillMaxWidth()
            .alpha(if (person.isHidden) 0.5f else 1f)
            .clickable { onPerson(person.id) }
            .padding(vertical = Spacing.xs),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.spacedBy(Spacing.xs),
    ) {
        FaceAvatar(person.cover?.faceId, 88.dp)
        Text(
            personLabel(person),
            style = MaterialTheme.typography.titleSmall,
            color = if (person.name == null) p.muted else p.text,
            maxLines = 1,
            overflow = TextOverflow.Ellipsis,
        )
        Text(
            pluralStringResource(R.plurals.search_photos, person.fileCount.toInt(), person.fileCount.toInt()),
            style = MaterialTheme.typography.labelSmall,
            color = p.muted,
        )
    }
}
