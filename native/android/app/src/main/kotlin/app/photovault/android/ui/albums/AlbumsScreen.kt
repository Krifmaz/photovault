@file:OptIn(ExperimentalMaterial3Api::class)

package app.photovault.android.ui.albums

import androidx.compose.foundation.background
import androidx.compose.foundation.border
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
import androidx.compose.foundation.lazy.grid.GridCells
import androidx.compose.foundation.lazy.grid.GridItemSpan
import androidx.compose.foundation.lazy.grid.LazyGridScope
import androidx.compose.foundation.lazy.grid.LazyVerticalGrid
import androidx.compose.foundation.lazy.grid.items
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBar
import androidx.compose.material3.TopAppBarDefaults
import androidx.compose.material3.pulltorefresh.PullToRefreshBox
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.res.pluralStringResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import app.photovault.android.R
import app.photovault.android.data.albums.AlbumDetails
import app.photovault.android.ui.theme.EncoreRadius
import app.photovault.android.ui.theme.Spacing
import app.photovault.android.ui.theme.palette

/** The Albums tab. */
@Composable
fun AlbumsScreen(
    vm: AlbumsViewModel,
    onOpenAlbum: (Long) -> Unit,
    onHidden: () -> Unit,
    onTrash: () -> Unit,
) {
    val state by vm.state.collectAsStateWithLifecycle()
    val p = MaterialTheme.palette
    val open by rememberUpdatedState(onOpenAlbum)
    var creating by rememberSaveable { mutableStateOf(false) }
    val snackbar = remember { SnackbarHostState() }

    // Back from an album (renamed, deleted, new photos): re-read core's index.
    LaunchedEffect(vm) { vm.refresh() }
    LaunchedEffect(vm) {
        vm.eventFlow.collect { e ->
            when (e) {
                is AlbumsEvent.Open -> {
                    creating = false
                    open(e.collectionId)
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

    Scaffold(
        containerColor = p.bg,
        snackbarHost = { SnackbarHost(snackbar) },
        topBar = {
            Column {
                TopAppBar(
                    colors = TopAppBarDefaults.topAppBarColors(containerColor = p.bg, titleContentColor = p.text),
                    title = {
                        Column {
                            Text(stringResource(R.string.albums_title), style = MaterialTheme.typography.titleLarge)
                            if (state.demo) {
                                Text(stringResource(R.string.timeline_demo), style = MaterialTheme.typography.bodySmall, color = p.muted)
                            }
                        }
                    },
                    actions = {
                        IconButton(onClick = { creating = true }, modifier = Modifier.testTag("album_new")) {
                            Icon(painterResource(R.drawable.ic_add), contentDescription = stringResource(R.string.albums_new), tint = p.text)
                        }
                    },
                )
                HorizontalDivider(color = p.border)
            }
        },
    ) { padding ->
        PullToRefreshBox(
            isRefreshing = state.syncing,
            onRefresh = vm::sync,
            modifier = Modifier.fillMaxSize().padding(padding),
        ) {
            if (!state.loaded) {
                Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) { CircularProgressIndicator(color = p.accent2) }
            } else {
                val mineTitle = stringResource(R.string.albums_mine)
                val sharedTitle = stringResource(R.string.albums_shared_with_me)
                LazyVerticalGrid(
                    columns = GridCells.Adaptive(150.dp),
                    contentPadding = PaddingValues(horizontal = Spacing.md, vertical = Spacing.md),
                    modifier = Modifier.fillMaxSize().testTag("album_grid"),
                ) {
                    item(key = "places", span = { GridItemSpan(maxLineSpan) }, contentType = "places") {
                        Row(
                            Modifier.fillMaxWidth().padding(horizontal = Spacing.xs, vertical = Spacing.xs),
                            horizontalArrangement = Arrangement.spacedBy(Spacing.md),
                        ) {
                            PlaceCard(
                                title = stringResource(R.string.hidden_title),
                                subtitle = stringResource(R.string.hidden_subtitle_short),
                                icon = R.drawable.ic_lock,
                                onClick = onHidden,
                                modifier = Modifier.weight(1f).testTag("albums_hidden"),
                            )
                            PlaceCard(
                                title = stringResource(R.string.trash_title),
                                subtitle = pluralStringResource(R.plurals.album_items, state.trashCount.toInt(), state.trashCount.toInt()),
                                icon = R.drawable.ic_delete,
                                onClick = onTrash,
                                modifier = Modifier.weight(1f).testTag("albums_trash"),
                            )
                        }
                    }
                    section("mine", mineTitle, state.mine, open)
                    section("shared", sharedTitle, state.shared, open)
                    if (state.mine.isEmpty() && state.shared.isEmpty()) {
                        item(key = "empty", span = { GridItemSpan(maxLineSpan) }, contentType = "empty") {
                            Text(
                                stringResource(R.string.albums_empty),
                                style = MaterialTheme.typography.bodyMedium,
                                color = p.muted,
                                textAlign = TextAlign.Center,
                                modifier = Modifier.fillMaxWidth().padding(Spacing.section),
                            )
                        }
                    }
                }
            }
        }
    }

    if (creating) {
        NameDialog(
            title = stringResource(R.string.albums_new),
            initial = "",
            confirm = stringResource(R.string.create),
            onConfirm = vm::createAlbum,
            onDismiss = { creating = false },
        )
    }
}

private fun LazyGridScope.section(key: String, title: String, albums: List<AlbumDetails>, onOpen: (Long) -> Unit) {
    if (albums.isEmpty()) return
    item(key = "h-$key", span = { GridItemSpan(maxLineSpan) }, contentType = "header") {
        Text(
            title,
            style = MaterialTheme.typography.titleMedium,
            color = MaterialTheme.palette.text,
            modifier = Modifier.padding(start = Spacing.xs, top = Spacing.xl, bottom = Spacing.xs),
        )
    }
    items(albums, key = { "a${it.id}" }, contentType = { "album" }) { a ->
        AlbumTile(a, onClick = { onOpen(a.id) })
    }
}

@Composable
private fun PlaceCard(title: String, subtitle: String, icon: Int, onClick: () -> Unit, modifier: Modifier = Modifier) {
    val p = MaterialTheme.palette
    val shape = RoundedCornerShape(EncoreRadius.tile)
    Row(
        modifier
            .clip(shape)
            .background(p.panel, shape)
            .border(1.dp, p.border, shape)
            .clickable(onClick = onClick)
            .padding(horizontal = Spacing.lg, vertical = Spacing.base),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(Spacing.base),
    ) {
        Icon(painterResource(icon), contentDescription = null, tint = p.accent2, modifier = Modifier.size(22.dp))
        Column {
            Text(title, style = MaterialTheme.typography.titleSmall, color = p.text)
            Text(subtitle, style = MaterialTheme.typography.bodySmall, color = p.muted)
        }
    }
}
