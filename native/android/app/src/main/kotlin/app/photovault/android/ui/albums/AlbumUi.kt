@file:OptIn(ExperimentalFoundationApi::class, ExperimentalMaterial3Api::class)

package app.photovault.android.ui.albums

import androidx.annotation.DrawableRes
import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.material3.TopAppBarDefaults
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.produceState
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.res.pluralStringResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import app.photovault.android.R
import app.photovault.android.data.FileKind
import app.photovault.android.data.TimelineItem
import app.photovault.android.data.albums.AlbumDetails
import app.photovault.android.ui.components.PvTextField
import app.photovault.android.ui.image.Thumb
import app.photovault.android.ui.nav.sharedPhoto
import app.photovault.android.ui.theme.EncoreRadius
import app.photovault.android.ui.theme.Spacing
import app.photovault.android.ui.theme.palette
import coil3.compose.AsyncImage

/** Display name of an album; the hidden-photos collection gets its own wording. */
@Composable
fun albumName(a: AlbumDetails): String = when {
    a.isDefaultHidden -> stringResource(R.string.hidden_photos)
    a.collection.kind == "uncategorized" -> stringResource(R.string.album_uncategorized)
    a.collection.kind == "favorites" -> stringResource(R.string.album_favorites)
    else -> a.collection.name.ifBlank { stringResource(R.string.album_untitled) }
}

/** An album in a grid: cover, name, count, and a badge when it is shared or has a link. */
@Composable
fun AlbumTile(album: AlbumDetails, onClick: () -> Unit, modifier: Modifier = Modifier) {
    val p = MaterialTheme.palette
    val shape = RoundedCornerShape(EncoreRadius.tile)
    Column(
        modifier
            .testTag("album_tile")
            .clip(shape)
            .clickable(onClick = onClick)
            .padding(Spacing.xs),
        verticalArrangement = Arrangement.spacedBy(Spacing.xxs),
    ) {
        Box(
            Modifier
                .fillMaxWidth()
                .aspectRatio(1f)
                .clip(shape)
                .background(p.panel),
        ) {
            val cover = album.collection.coverFileId
            if (cover != null) {
                AsyncImage(
                    model = Thumb(cover),
                    contentDescription = null,
                    contentScale = ContentScale.Crop,
                    modifier = Modifier.fillMaxSize(),
                )
            } else {
                Icon(
                    painterResource(R.drawable.ic_albums),
                    contentDescription = null,
                    tint = p.faint,
                    modifier = Modifier.align(Alignment.Center).size(36.dp),
                )
            }
            val badge = when {
                !album.owned || album.sharees.isNotEmpty() -> R.drawable.ic_people
                album.publicLink != null -> R.drawable.ic_link
                album.hidden -> R.drawable.ic_lock
                else -> null
            }
            if (badge != null) {
                Box(
                    Modifier
                        .align(Alignment.TopEnd)
                        .padding(Spacing.sm)
                        .size(26.dp)
                        .clip(CircleShape)
                        .background(Color.Black.copy(alpha = 0.45f)),
                    contentAlignment = Alignment.Center,
                ) {
                    Icon(painterResource(badge), contentDescription = null, tint = Color.White, modifier = Modifier.size(16.dp))
                }
            }
        }
        Text(
            albumName(album),
            style = MaterialTheme.typography.titleSmall,
            color = p.text,
            maxLines = 1,
            overflow = TextOverflow.Ellipsis,
            modifier = Modifier.padding(top = Spacing.xs),
        )
        val count = album.collection.count.toInt()
        val sub = if (album.owned) {
            pluralStringResource(R.plurals.album_items, count, count)
        } else {
            stringResource(R.string.album_count_by, pluralStringResource(R.plurals.album_items, count, count), album.ownerEmail)
        }
        Text(sub, style = MaterialTheme.typography.bodySmall, color = p.muted, maxLines = 1, overflow = TextOverflow.Ellipsis)
    }
}

/**
 * A square photo that can be selected: tap opens (or toggles while selecting), long press starts selecting.
 * [label] is an optional caption in the corner (the trash's days left).
 */
@Composable
fun SelectablePhotoCell(
    item: TimelineItem,
    selected: Boolean,
    selecting: Boolean,
    onOpen: () -> Unit,
    onToggle: () -> Unit,
    modifier: Modifier = Modifier,
    label: String? = null,
    shared: Boolean = true,
) {
    val p = MaterialTheme.palette
    Box(
        modifier
            .testTag("photo_tile")
            .aspectRatio(1f)
            .padding(1.dp)
            .background(p.panel)
            .combinedClickable(
                onClick = { if (selecting) onToggle() else onOpen() },
                onLongClick = onToggle,
            ),
    ) {
        AsyncImage(
            model = Thumb(item.id),
            contentDescription = null,
            contentScale = ContentScale.Crop,
            modifier = Modifier
                .fillMaxSize()
                .then(if (shared) Modifier.sharedPhoto(item.id) else Modifier)
                .then(if (selected) Modifier.padding(Spacing.md).clip(RoundedCornerShape(EncoreRadius.small)) else Modifier),
        )
        if (item.kind == FileKind.Video) {
            CornerBadge(Modifier.align(Alignment.BottomEnd)) {
                Icon(painterResource(R.drawable.ic_play), contentDescription = null, tint = Color.White, modifier = Modifier.size(12.dp))
            }
        }
        if (label != null) {
            CornerBadge(Modifier.align(Alignment.BottomStart)) {
                Text(label, color = Color.White, style = MaterialTheme.typography.labelSmall)
            }
        }
        if (selecting) {
            Box(
                Modifier
                    .align(Alignment.TopStart)
                    .padding(Spacing.xs)
                    .size(22.dp)
                    .clip(CircleShape)
                    .background(if (selected) p.accent2 else Color.Black.copy(alpha = 0.3f))
                    .border(1.5.dp, Color.White, CircleShape),
                contentAlignment = Alignment.Center,
            ) {
                if (selected) Icon(painterResource(R.drawable.ic_check_circle), contentDescription = null, tint = p.onAccent, modifier = Modifier.size(18.dp))
            }
        }
    }
}

@Composable
private fun CornerBadge(modifier: Modifier, content: @Composable () -> Unit) {
    Row(
        modifier
            .padding(Spacing.xs)
            .clip(RoundedCornerShape(EncoreRadius.small))
            .background(Color.Black.copy(alpha = 0.45f))
            .padding(horizontal = Spacing.xs, vertical = Spacing.xxs),
        verticalAlignment = Alignment.CenterVertically,
    ) { content() }
}

/** Top bar shared by the album screens: back, title + subtitle, actions. */
@Composable
fun AlbumTopBar(
    title: String,
    subtitle: String?,
    onBack: () -> Unit,
    @DrawableRes navIcon: Int = R.drawable.ic_arrow_back,
    actions: @Composable () -> Unit = {},
) {
    val p = MaterialTheme.palette
    Column {
        TopAppBar(
            colors = TopAppBarDefaults.topAppBarColors(containerColor = p.bg, titleContentColor = p.text),
            navigationIcon = {
                IconButton(onClick = onBack) {
                    Icon(painterResource(navIcon), contentDescription = stringResource(R.string.back), tint = p.text)
                }
            },
            title = {
                Column {
                    Text(title, style = MaterialTheme.typography.titleLarge, maxLines = 1, overflow = TextOverflow.Ellipsis)
                    if (subtitle != null) {
                        Text(subtitle, style = MaterialTheme.typography.bodySmall, color = p.muted, maxLines = 1, overflow = TextOverflow.Ellipsis)
                    }
                }
            },
            actions = { actions() },
        )
        HorizontalDivider(color = p.border)
    }
}

/** Ask for a name (new album, rename). */
@Composable
fun NameDialog(title: String, initial: String, confirm: String, onConfirm: (String) -> Unit, onDismiss: () -> Unit) {
    var name by rememberSaveable { mutableStateOf(initial) }
    val focus = remember { FocusRequester() }
    LaunchedEffect(Unit) { runCatching { focus.requestFocus() } }
    val p = MaterialTheme.palette
    AlertDialog(
        onDismissRequest = onDismiss,
        containerColor = p.bg2,
        title = { Text(title, color = p.text) },
        text = {
            PvTextField(
                value = name,
                onValueChange = { name = it },
                label = stringResource(R.string.album_name),
                imeAction = ImeAction.Done,
                onIme = { if (name.isNotBlank()) onConfirm(name.trim()) },
                modifier = Modifier.focusRequester(focus),
            )
        },
        confirmButton = {
            TextButton(onClick = { onConfirm(name.trim()) }, enabled = name.isNotBlank()) {
                Text(confirm, color = p.accent2)
            }
        },
        dismissButton = { TextButton(onClick = onDismiss) { Text(stringResource(R.string.cancel), color = p.muted) } },
    )
}

/** A yes/no question; [danger] colours the confirm button as destructive. */
@Composable
fun ConfirmDialog(
    title: String,
    body: String?,
    confirm: String,
    onConfirm: () -> Unit,
    onDismiss: () -> Unit,
    danger: Boolean = false,
) {
    val p = MaterialTheme.palette
    AlertDialog(
        onDismissRequest = onDismiss,
        containerColor = p.bg2,
        title = { Text(title, color = p.text) },
        text = { if (body != null) Text(body, color = p.muted) },
        confirmButton = { TextButton(onClick = onConfirm) { Text(confirm, color = if (danger) p.err else p.accent2) } },
        dismissButton = { TextButton(onClick = onDismiss) { Text(stringResource(R.string.cancel), color = p.muted) } },
    )
}

/**
 * Pick an album. [loadTargets] runs once when the dialog opens. [noneLabel] adds a first row that picks
 * null (Uncategorized); [allowNew] adds "New album…", which asks for a name and calls [onNew].
 */
@Composable
fun AlbumPickerDialog(
    title: String,
    loadTargets: suspend () -> List<AlbumDetails>,
    onPick: (Long?) -> Unit,
    onDismiss: () -> Unit,
    noneLabel: String? = null,
    allowNew: Boolean = false,
    onNew: (String) -> Unit = {},
) {
    val p = MaterialTheme.palette
    val targets by produceState<List<AlbumDetails>?>(null) { value = loadTargets() }
    var naming by remember { mutableStateOf(false) }
    if (naming) {
        NameDialog(
            title = stringResource(R.string.albums_new),
            initial = "",
            confirm = stringResource(R.string.create),
            onConfirm = { onNew(it) },
            onDismiss = { naming = false },
        )
        return
    }
    AlertDialog(
        onDismissRequest = onDismiss,
        containerColor = p.bg2,
        title = { Text(title, color = p.text) },
        text = {
            LazyColumn(Modifier.heightIn(max = 420.dp)) {
                if (allowNew) {
                    item(key = "new") { PickerRow(stringResource(R.string.albums_new_ellipsis), R.drawable.ic_add) { naming = true } }
                }
                if (noneLabel != null) {
                    item(key = "none") { PickerRow(noneLabel, R.drawable.ic_albums) { onPick(null) } }
                }
                items(targets.orEmpty(), key = { it.id }) { a ->
                    PickerRow(albumName(a), R.drawable.ic_albums) { onPick(a.id) }
                }
            }
        },
        confirmButton = {},
        dismissButton = { TextButton(onClick = onDismiss) { Text(stringResource(R.string.cancel), color = p.muted) } },
    )
}

@Composable
private fun PickerRow(text: String, @DrawableRes icon: Int, onClick: () -> Unit) {
    val p = MaterialTheme.palette
    Row(
        Modifier
            .fillMaxWidth()
            .clip(RoundedCornerShape(EncoreRadius.control))
            .clickable(onClick = onClick)
            .padding(horizontal = Spacing.md, vertical = Spacing.lg),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(Spacing.lg),
    ) {
        Icon(painterResource(icon), contentDescription = null, tint = p.accent2, modifier = Modifier.size(20.dp))
        Text(text, color = p.text, style = MaterialTheme.typography.bodyLarge, maxLines = 1, overflow = TextOverflow.Ellipsis)
    }
}
