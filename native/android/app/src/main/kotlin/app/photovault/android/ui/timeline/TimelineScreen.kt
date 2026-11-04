@file:OptIn(ExperimentalMaterial3Api::class, ExperimentalFoundationApi::class)

package app.photovault.android.ui.timeline

import androidx.compose.animation.core.animate
import androidx.compose.animation.core.spring
import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.gestures.awaitEachGesture
import androidx.compose.foundation.gestures.awaitFirstDown
import androidx.compose.foundation.gestures.calculateCentroid
import androidx.compose.foundation.gestures.calculateZoom
import androidx.compose.foundation.gestures.scrollBy
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.grid.GridCells
import androidx.compose.foundation.lazy.grid.LazyGridState
import androidx.compose.foundation.lazy.grid.LazyVerticalGrid
import androidx.compose.foundation.lazy.grid.rememberLazyGridState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBar
import androidx.compose.material3.TopAppBarDefaults
import androidx.compose.material3.pulltorefresh.PullToRefreshBox
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.Stable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.setValue
import androidx.compose.runtime.snapshotFlow
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.TransformOrigin
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.input.pointer.PointerEventPass
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.input.pointer.positionChanged
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import app.photovault.android.R
import app.photovault.android.data.FileKind
import app.photovault.android.ui.image.Thumb
import app.photovault.android.ui.nav.sharedPhoto
import app.photovault.android.ui.theme.EncoreRadius
import app.photovault.android.ui.theme.Spacing
import app.photovault.android.ui.theme.palette
import coil3.SingletonImageLoader
import coil3.compose.AsyncImage
import coil3.request.ImageRequest
import coil3.size.Scale
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.distinctUntilChanged
import kotlinx.coroutines.launch
import kotlin.math.abs

/** The photo timeline: sticky date headers, pinch between densities, tap to open. */
@Composable
fun TimelineScreen(
    vm: TimelineViewModel,
    returnIndex: Flow<Int?>,
    onReturnConsumed: () -> Unit,
    onOpen: (index: Int, fileId: Long) -> Unit,
    onSignedOut: () -> Unit,
    onSwitchServer: () -> Unit,
) {
    val signedOut by rememberUpdatedState(onSignedOut)
    val switchServer by rememberUpdatedState(onSwitchServer)
    LaunchedEffect(vm) {
        vm.eventFlow.collect { e ->
            when (e) {
                TimelineEvent.SignedOut -> signedOut()
                TimelineEvent.SwitchServer -> switchServer()
            }
        }
    }
    val gridState = rememberLazyGridState()
    val zoom = remember { GridZoomState() }

    // Back from the viewer: make sure the photo it ended on is on screen.
    LaunchedEffect(returnIndex) {
        returnIndex.collect { index ->
            if (index == null) return@collect
            onReturnConsumed()
            val model = vm.grid.value
            val entry = model.entryIndexOfTimeline.getOrNull(index) ?: return@collect
            val visible = gridState.layoutInfo.visibleItemsInfo
            val first = visible.firstOrNull()?.index ?: 0
            val last = visible.lastOrNull()?.index ?: 0
            if (entry !in (first + 1) until last) gridState.scrollToItem(entry)
        }
    }

    Scaffold(
        containerColor = MaterialTheme.palette.bg,
        topBar = { TimelineTopBar(vm, gridState) },
    ) { padding ->
        RefreshBox(vm) {
            PhotoGrid(vm, gridState, zoom, padding, onOpen)
            StatusOverlay(vm, Modifier.padding(padding))
        }
    }
}

@Composable
private fun RefreshBox(vm: TimelineViewModel, content: @Composable () -> Unit) {
    val status by vm.status.collectAsStateWithLifecycle()
    PullToRefreshBox(
        isRefreshing = status.syncing && status.loaded,
        onRefresh = vm::sync,
        modifier = Modifier.fillMaxSize(),
    ) {
        content()
    }
}

@Composable
private fun TimelineTopBar(vm: TimelineViewModel, gridState: LazyGridState) {
    val status by vm.status.collectAsStateWithLifecycle()
    val p = MaterialTheme.palette
    var menu by remember { mutableStateOf(false) }
    Column(
        Modifier.drawBehind {
            // scroll-dependent divider: read in the draw phase only, no recomposition while scrolling
            if (gridState.firstVisibleItemIndex > 0 || gridState.firstVisibleItemScrollOffset > 0) {
                drawRect(p.border, topLeft = Offset(0f, size.height - 1.dp.toPx()))
            }
        },
    ) {
        TopAppBar(
            colors = TopAppBarDefaults.topAppBarColors(containerColor = p.bg, titleContentColor = p.text),
            title = {
                Column {
                    Text(stringResource(R.string.timeline_title), style = MaterialTheme.typography.titleLarge)
                    val sub = when {
                        status.demo -> stringResource(R.string.timeline_demo)
                        status.syncing -> stringResource(R.string.timeline_syncing)
                        status.syncError != null -> stringResource(R.string.timeline_sync_failed, status.syncError.orEmpty())
                        else -> status.serverName
                    }
                    if (sub != null) {
                        Text(sub, style = MaterialTheme.typography.bodySmall, color = p.muted, maxLines = 1, overflow = TextOverflow.Ellipsis)
                    }
                }
            },
            actions = {
                IconButton(onClick = { menu = true }) {
                    Icon(painterResource(R.drawable.ic_account), contentDescription = stringResource(R.string.timeline_account), tint = p.text)
                }
                DropdownMenu(expanded = menu, onDismissRequest = { menu = false }) {
                    status.account?.let { account ->
                        Text(
                            account.email,
                            style = MaterialTheme.typography.labelMedium,
                            color = p.muted,
                            modifier = Modifier.padding(horizontal = Spacing.xl, vertical = Spacing.md),
                        )
                        HorizontalDivider(color = p.border)
                    }
                    DropdownMenuItem(
                        text = { Text(stringResource(R.string.timeline_sync_now)) },
                        leadingIcon = { Icon(painterResource(R.drawable.ic_refresh), contentDescription = null) },
                        onClick = { menu = false; vm.sync() },
                    )
                    DropdownMenuItem(
                        text = { Text(stringResource(R.string.timeline_switch_server)) },
                        leadingIcon = { Icon(painterResource(R.drawable.ic_server), contentDescription = null) },
                        onClick = { menu = false; vm.switchServer() },
                    )
                    DropdownMenuItem(
                        text = { Text(stringResource(R.string.timeline_sign_out)) },
                        leadingIcon = { Icon(painterResource(R.drawable.ic_logout), contentDescription = null) },
                        onClick = { menu = false; vm.signOut() },
                    )
                }
            },
        )
        if (status.syncing) {
            LinearProgressIndicator(Modifier.fillMaxWidth(), color = p.accent2, trackColor = p.panel)
        }
    }
}

@Composable
private fun StatusOverlay(vm: TimelineViewModel, modifier: Modifier) {
    val status by vm.status.collectAsStateWithLifecycle()
    val p = MaterialTheme.palette
    Box(modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
        when {
            !status.loaded -> CircularProgressIndicator(color = p.accent2)
            status.empty && !status.syncing -> Column(
                horizontalAlignment = Alignment.CenterHorizontally,
                verticalArrangement = Arrangement.spacedBy(Spacing.md),
                modifier = Modifier.padding(Spacing.section),
            ) {
                Text(stringResource(R.string.timeline_empty_title), style = MaterialTheme.typography.titleLarge, color = p.text)
                Text(
                    status.syncError ?: stringResource(R.string.timeline_empty_body),
                    style = MaterialTheme.typography.bodyMedium,
                    color = p.muted,
                    textAlign = TextAlign.Center,
                )
            }
        }
    }
}

/** Pinch state, read only in the graphics layer so a pinch never recomposes the grid. */
@Stable
class GridZoomState {
    var scale by mutableFloatStateOf(1f)
    var pivot by mutableStateOf(Offset.Zero)

    /** Set when a pinch changed the level: which cell to keep under the fingers once the new grid is laid out. */
    var anchorTimelineIndex: Int = -1
    var anchorFraction: Float = 0f
    var anchorZoomIndex: Int = -1
}

private const val COMMIT_IN = 1.25f
private const val COMMIT_OUT = 0.8f

@Composable
private fun PhotoGrid(
    vm: TimelineViewModel,
    gridState: LazyGridState,
    zoom: GridZoomState,
    contentPadding: PaddingValues,
    onOpen: (Int, Long) -> Unit,
) {
    val model by vm.grid.collectAsStateWithLifecycle()
    val scope = rememberCoroutineScope()
    val context = LocalContext.current
    val p = MaterialTheme.palette

    // After a level change: put the anchored photo back under the fingers.
    LaunchedEffect(model) {
        val anchor = zoom.anchorTimelineIndex
        if (anchor < 0 || model.zoomIndex != zoom.anchorZoomIndex) return@LaunchedEffect
        zoom.anchorTimelineIndex = -1
        val entry = model.entryIndexOfTimeline.getOrNull(anchor) ?: return@LaunchedEffect
        val info = gridState.layoutInfo
        val cellHeight = info.viewportSize.width.toFloat() / model.columns
        val top = info.beforeContentPadding
        val wantTop = zoom.pivot.y - zoom.anchorFraction * cellHeight
        // scrollToItem puts the anchor's row at the top; then move it down to where the fingers are
        gridState.scrollToItem(entry)
        if (wantTop > top) gridState.scrollBy(-(wantTop - top))
    }

    // Paging and thumbnail prefetch ahead of the scroll.
    LaunchedEffect(gridState, vm) {
        val loader = SingletonImageLoader.get(context)
        val prefetched = LinkedHashSet<Long>()
        snapshotFlow { gridState.layoutInfo.visibleItemsInfo.lastOrNull()?.index ?: -1 }
            .distinctUntilChanged()
            .collect { last ->
                if (last < 0) return@collect
                val m = vm.grid.value
                val entries = m.entries
                var i = last.coerceAtMost(entries.lastIndex)
                while (i >= 0 && entries[i] !is GridEntry.Cell) i--
                if (i >= 0) vm.onVisible((entries[i] as GridEntry.Cell).timelineIndex)
                val cellPx = gridState.layoutInfo.visibleItemsInfo.firstOrNull { it.contentType == GridContentType.CELL }?.size?.width
                    ?: return@collect
                val ahead = m.columns * PREFETCH_ROWS
                for (j in (last + 1)..minOf(entries.lastIndex, last + ahead)) {
                    val cell = entries[j] as? GridEntry.Cell ?: continue
                    if (!prefetched.add(cell.item.id)) continue
                    // Same scale as the cell (ContentScale.Crop = FILL) and at least its size (the cell's image is
                    // the slot minus its 1 dp padding), so the cell's memory-cache lookup accepts this bitmap
                    // instead of decrypting and decoding the thumbnail a second time.
                    loader.enqueue(
                        ImageRequest.Builder(context)
                            .data(Thumb(cell.item.id))
                            .size(cellPx, cellPx)
                            .scale(Scale.FILL)
                            .build(),
                    )
                }
                while (prefetched.size > PREFETCH_MEMORY) prefetched.remove(prefetched.first())
            }
    }

    val open by rememberUpdatedState(onOpen)
    Box(
        Modifier
            .fillMaxSize()
            .background(p.bg)
            .pointerInput(vm) {
                awaitEachGesture {
                    awaitFirstDown(requireUnconsumed = false, pass = PointerEventPass.Initial)
                    var pinched = false
                    do {
                        val event = awaitPointerEvent(PointerEventPass.Initial)
                        if (event.changes.count { it.pressed } >= 2) {
                            if (!pinched) {
                                pinched = true
                                zoom.pivot = event.calculateCentroid(useCurrent = true)
                            }
                            zoom.scale *= event.calculateZoom()
                            val before = vm.currentZoomIndex
                            val step = when {
                                zoom.scale > COMMIT_IN && before > 0 -> -1
                                zoom.scale < COMMIT_OUT && before < ZoomLevels.lastIndex -> 1
                                else -> 0
                            }
                            if (step != 0) {
                                // keep the photo under the fingers in place across the change
                                val cell = gridState.layoutInfo.visibleItemsInfo.firstOrNull {
                                    it.contentType == GridContentType.CELL &&
                                        zoom.pivot.y >= it.offset.y && zoom.pivot.y < it.offset.y + it.size.height &&
                                        zoom.pivot.x >= it.offset.x && zoom.pivot.x < it.offset.x + it.size.width
                                }
                                val entry = cell?.let { vm.grid.value.entries.getOrNull(it.index) as? GridEntry.Cell }
                                if (cell != null && entry != null) {
                                    zoom.anchorTimelineIndex = entry.timelineIndex
                                    zoom.anchorFraction = (zoom.pivot.y - cell.offset.y) / cell.size.height.coerceAtLeast(1)
                                    zoom.anchorZoomIndex = before + step
                                }
                                if (vm.zoomBy(step)) {
                                    val oldCols = ZoomLevels[before].columns.toFloat()
                                    val newCols = ZoomLevels[before + step].columns.toFloat()
                                    zoom.scale *= newCols / oldCols
                                }
                            } else {
                                // at the ends, a little give and no further
                                zoom.scale = zoom.scale.coerceIn(0.7f, 1.4f)
                            }
                            event.changes.forEach { if (it.positionChanged()) it.consume() }
                        }
                    } while (event.changes.any { it.pressed })
                    if (pinched) {
                        val from = zoom.scale
                        scope.launch {
                            animate(from, 1f, animationSpec = spring(dampingRatio = 0.85f, stiffness = 500f)) { v, _ -> zoom.scale = v }
                        }
                    }
                }
            }
            .graphicsLayer {
                val s = zoom.scale
                scaleX = s
                scaleY = s
                val pv = zoom.pivot
                transformOrigin = if (size.width > 0f && size.height > 0f) {
                    TransformOrigin(pv.x / size.width, pv.y / size.height)
                } else {
                    TransformOrigin.Center
                }
            },
    ) {
        val entries = model.entries
        LazyVerticalGrid(
            columns = GridCells.Fixed(model.columns),
            state = gridState,
            contentPadding = contentPadding,
            modifier = Modifier.fillMaxSize(),
        ) {
            var i = 0
            while (i < entries.size) {
                val header = entries[i] as? GridEntry.Header
                val start: Int
                if (header != null) {
                    stickyHeader(key = header.key, contentType = GridContentType.HEADER) {
                        HeaderRow(header.label)
                    }
                    start = i + 1
                } else {
                    start = i
                }
                var end = start
                while (end < entries.size && entries[end] is GridEntry.Cell) end++
                val from = start
                items(
                    count = end - from,
                    key = { entries[from + it].key },
                    contentType = { GridContentType.CELL },
                ) { offset ->
                    val cell = entries[from + offset] as GridEntry.Cell
                    PhotoCell(cell, open)
                }
                i = end
            }
        }
    }
}

private const val PREFETCH_ROWS = 6
private const val PREFETCH_MEMORY = 1500

@Composable
private fun HeaderRow(label: String) {
    val p = MaterialTheme.palette
    Text(
        label,
        style = MaterialTheme.typography.titleMedium,
        color = p.text,
        maxLines = 1,
        overflow = TextOverflow.Ellipsis,
        modifier = Modifier
            .fillMaxWidth()
            .background(p.bg.copy(alpha = 0.94f))
            .padding(horizontal = Spacing.lg, vertical = Spacing.base),
    )
}

@Composable
private fun PhotoCell(cell: GridEntry.Cell, onOpen: (Int, Long) -> Unit) {
    val p = MaterialTheme.palette
    val item = cell.item
    Box(
        Modifier
            .aspectRatio(1f)
            .padding(1.dp)
            .background(p.panel)
            .clickable { onOpen(cell.timelineIndex, item.id) },
    ) {
        AsyncImage(
            model = Thumb(item.id),
            contentDescription = null,
            contentScale = ContentScale.Crop,
            modifier = Modifier.fillMaxSize().sharedPhoto(item.id),
        )
        when (item.kind) {
            FileKind.Video -> Badge(Modifier.align(Alignment.BottomEnd)) {
                Icon(painterResource(R.drawable.ic_play), contentDescription = null, tint = Color.White, modifier = Modifier.size(12.dp))
                item.durationMs?.let { Text(formatDuration(it), color = Color.White, style = MaterialTheme.typography.labelSmall) }
            }
            FileKind.LivePhoto -> Badge(Modifier.align(Alignment.TopEnd)) {
                Icon(painterResource(R.drawable.ic_live), contentDescription = null, tint = Color.White, modifier = Modifier.size(12.dp))
            }
            else -> Unit
        }
    }
}

@Composable
private fun Badge(modifier: Modifier, content: @Composable () -> Unit) {
    Row(
        modifier
            .padding(Spacing.xs)
            .clip(RoundedCornerShape(EncoreRadius.small))
            .background(Color.Black.copy(alpha = 0.45f))
            .padding(horizontal = Spacing.xs, vertical = Spacing.xxs),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(Spacing.xxs),
    ) { content() }
}

internal fun formatDuration(ms: Int): String {
    val total = abs(ms) / 1000
    val h = total / 3600
    val m = (total % 3600) / 60
    val s = total % 60
    return if (h > 0) "%d:%02d:%02d".format(h, m, s) else "%d:%02d".format(m, s)
}
