package app.photovault.android.ui.map

import android.graphics.Paint
import android.graphics.Rect
import android.graphics.RectF
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.gestures.awaitEachGesture
import androidx.compose.foundation.gestures.awaitFirstDown
import androidx.compose.foundation.gestures.calculateCentroid
import androidx.compose.foundation.gestures.calculatePan
import androidx.compose.foundation.gestures.calculateZoom
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyRow
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.snapshotFlow
import androidx.compose.runtime.snapshots.Snapshot
import androidx.compose.runtime.structuralEqualityPolicy
import androidx.compose.runtime.derivedStateOf
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.drawscope.drawIntoCanvas
import androidx.compose.ui.graphics.nativeCanvas
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.input.pointer.positionChanged
import androidx.compose.ui.input.pointer.util.VelocityTracker
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.layout.onSizeChanged
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.res.pluralStringResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.IntOffset
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import app.photovault.android.R
import app.photovault.android.ui.image.Thumb
import app.photovault.android.ui.theme.EncoreRadius
import app.photovault.android.ui.theme.Spacing
import app.photovault.android.ui.theme.palette
import coil3.compose.AsyncImage
import kotlinx.coroutines.FlowPreview
import kotlinx.coroutines.flow.debounce
import kotlinx.coroutines.flow.distinctUntilChanged
import kotlinx.coroutines.launch
import kotlin.math.ceil
import kotlin.math.floor
import kotlin.math.max
import kotlin.math.min

private val MARKER = 52.dp

/**
 * Photos on a map, like Ente's map screen: OpenStreetMap tiles, photos grouped into clusters that show the
 * newest photo and a count, and the photos of the visible area in a strip at the bottom. Tapping a cluster
 * zooms into it; tapping a photo opens it.
 */
@OptIn(FlowPreview::class)
@Composable
fun MapScreen(vm: MapViewModel, onOpen: (fileId: Long) -> Unit, onBack: () -> Unit) {
    val context = LocalContext.current
    val tiles = remember { TileSource(context.applicationContext) }
    DisposableEffect(tiles) { onDispose { tiles.close() } }
    val map = rememberSaveable(saver = MapState.Saver) { MapState(MapState.WORLD, false) }
    val state by vm.state.collectAsStateWithLifecycle()
    val clusters by vm.clusters.collectAsStateWithLifecycle()
    val inView by vm.inView.collectAsStateWithLifecycle()
    val p = MaterialTheme.palette
    val density = LocalDensity.current
    val markerPx = with(density) { MARKER.toPx() }
    val scope = rememberCoroutineScope()

    // First view: all the photos.
    LaunchedEffect(state.loaded, map.size) {
        if (map.placed || !state.loaded || map.size.width == 0) return@LaunchedEffect
        val b = state.bounds
        if (b == null) map.place(MapState.WORLD)
        else map.place(MapMath.fit(b[0], b[1], b[2], b[3], map.size.width.toFloat(), map.size.height.toFloat(), markerPx))
    }
    // Clusters for the integer zoom shown.
    LaunchedEffect(state.points) {
        snapshotFlow { MapState.clusterZoom(map.camera.zoom) }
            .distinctUntilChanged()
            .collect { vm.clustersFor(it, markerPx * 1.15f) }
    }
    // The photo strip follows the map once it settles.
    LaunchedEffect(state.points) {
        snapshotFlow { if (map.placed && map.size.width > 0) map.camera else null }
            .distinctUntilChanged()
            .debounce(250)
            .collect { cam -> if (cam != null) map.viewBounds().let { vm.viewChanged(it[0], it[1], it[2], it[3]) } }
    }

    Box(Modifier.fillMaxSize().background(p.panel)) {
        Box(
            Modifier
                .fillMaxSize()
                .onSizeChanged { map.size = it }
                .pointerInput(map) { mapGestures(map) }
                .pointerInput(map) {
                    detectTapGestures(onDoubleTap = { pos -> scope.launch { map.zoomBy(1f, pos) } })
                }
                .testTag("map"),
        ) {
        TileLayer(map, tiles, Modifier.fillMaxSize())
        Markers(map, clusters.second, markerPx) { c ->
            if (c.isSpot || map.camera.zoom >= MapMath.MAX_ZOOM - 0.5f) {
                vm.showSpot(c)
            } else {
                scope.launch {
                    val target = if (c.count == 1) Camera(c.x, c.y, max(map.camera.zoom + 2f, 14f))
                    else MapMath.fit(c.minX, c.minY, c.maxX, c.maxY, map.size.width.toFloat(), map.size.height.toFloat(), markerPx * 1.5f, MapMath.MAX_ZOOM)
                    map.animateTo(target.copy(zoom = max(target.zoom, map.camera.zoom + 1f).coerceAtMost(MapMath.MAX_ZOOM)))
                }
            }
        }
        }
        // top bar: back, title
        Row(
            Modifier.statusBarsPadding().padding(Spacing.md),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            IconButton(
                onClick = onBack,
                modifier = Modifier.clip(CircleShape).background(p.bg2).testTag("map_back"),
            ) {
                Icon(painterResource(R.drawable.ic_arrow_back), contentDescription = stringResource(R.string.back), tint = p.text)
            }
        }
        Column(Modifier.align(Alignment.BottomCenter).fillMaxWidth()) {
            Text(
                stringResource(R.string.map_attribution),
                style = MaterialTheme.typography.labelSmall,
                color = Color(0xFF333333),
                modifier = Modifier
                    .align(Alignment.End)
                    .padding(end = Spacing.xs, bottom = Spacing.xs)
                    .background(Color.White.copy(alpha = 0.75f), RoundedCornerShape(EncoreRadius.small))
                    .padding(horizontal = Spacing.sm, vertical = Spacing.xxs),
            )
            PhotoStrip(
                loaded = state.loaded,
                anyPhotos = state.points.isNotEmpty(),
                photos = inView,
                onOpen = onOpen,
            )
        }
    }
}

@Composable
private fun TileLayer(map: MapState, tiles: TileSource, modifier: Modifier) {
    val tick = remember { mutableIntStateOf(0) }
    val paint = remember { Paint(Paint.FILTER_BITMAP_FLAG or Paint.ANTI_ALIAS_FLAG) }
    val src = remember { Rect() }
    val dst = remember { RectF() }
    val scope = rememberCoroutineScope()

    // Ask for the visible tiles whenever the view changes; drop requests for tiles that scrolled away.
    LaunchedEffect(tiles) {
        snapshotFlow { visibleTiles(map) }
            .distinctUntilChanged()
            .collect { keys ->
                tiles.retain(keys.mapTo(HashSet()) { it.packed })
                // centre first: the eye goes there
                for (k in keys) tiles.request(k) { scope.launch { tick.intValue++ } }
            }
    }

    Canvas(modifier) {
        @Suppress("UNUSED_VARIABLE") val redraw = tick.intValue
        val cam = map.camera
        val z = MapMath.tileZoom(cam.zoom)
        val n = 1 shl z
        val ws = MapMath.worldSize(cam.zoom)
        val tilePx = ws / n
        val left = cam.x * ws - size.width / 2.0
        val top = cam.y * ws - size.height / 2.0
        val x0 = floor(left / tilePx).toInt()
        val x1 = floor((left + size.width) / tilePx).toInt()
        val y0 = max(0, floor(top / tilePx).toInt())
        val y1 = min(n - 1, floor((top + size.height) / tilePx).toInt())
        drawIntoCanvas { c ->
            val canvas = c.nativeCanvas
            for (ty in y0..y1) for (tx in x0..x1) {
                val wx = Math.floorMod(tx, n)
                dst.set(
                    floor(tx * tilePx - left).toFloat(),
                    floor(ty * tilePx - top).toFloat(),
                    ceil((tx + 1) * tilePx - left).toFloat(),
                    ceil((ty + 1) * tilePx - top).toFloat(),
                )
                val exact = tiles.cached(TileKey(z, wx, ty))
                if (exact != null) {
                    canvas.drawBitmap(exact, null, dst, paint)
                    continue
                }
                // Not loaded yet: stretch the nearest loaded ancestor so the map never flashes blank.
                for (k in 1..min(z, 5)) {
                    val parent = tiles.cached(TileKey(z - k, wx shr k, ty shr k)) ?: continue
                    val sub = MapMath.TILE shr k
                    val sx = (wx and ((1 shl k) - 1)) * sub
                    val sy = (ty and ((1 shl k) - 1)) * sub
                    src.set(sx, sy, sx + sub, sy + sub)
                    canvas.drawBitmap(parent, src, dst, paint)
                    break
                }
            }
        }
    }
}

/** The tiles the camera shows, centre first. */
private fun visibleTiles(map: MapState): List<TileKey> {
    val size = map.size
    if (size.width == 0 || !map.placed) return emptyList()
    val cam = map.camera
    val z = MapMath.tileZoom(cam.zoom)
    val n = 1 shl z
    val ws = MapMath.worldSize(cam.zoom)
    val tilePx = ws / n
    val left = cam.x * ws - size.width / 2.0
    val top = cam.y * ws - size.height / 2.0
    val x0 = floor(left / tilePx).toInt()
    val x1 = floor((left + size.width) / tilePx).toInt()
    val y0 = max(0, floor(top / tilePx).toInt())
    val y1 = min(n - 1, floor((top + size.height) / tilePx).toInt())
    val cx = (x0 + x1) / 2.0
    val cy = (y0 + y1) / 2.0
    val out = ArrayList<TileKey>()
    for (ty in y0..y1) for (tx in x0..x1) out += TileKey(z, Math.floorMod(tx, n), ty)
    // same tile twice when the world is narrower than the screen
    return out.distinct().sortedBy { (it.x - cx) * (it.x - cx) + (it.y - cy) * (it.y - cy) }
}

/** Pan with one or more fingers, pinch to zoom around the fingers, fling on release. */
private suspend fun androidx.compose.ui.input.pointer.PointerInputScope.mapGestures(map: MapState) {
    kotlinx.coroutines.coroutineScope {
        awaitEachGesture {
            val down = awaitFirstDown(requireUnconsumed = false)
            launch { map.stop() }
            val tracker = VelocityTracker()
            tracker.addPosition(down.uptimeMillis, down.position)
            var pointers = 1
            do {
                val event = awaitPointerEvent()
                if (event.changes.any { it.isConsumed }) break
                val pressed = event.changes.count { it.pressed }
                if (pressed != pointers) {
                    // a finger came or went: the centroid jumps, so velocity restarts from here
                    tracker.resetTracking()
                    pointers = pressed
                }
                val zoom = event.calculateZoom()
                val pan = event.calculatePan()
                val focus = event.calculateCentroid(useCurrent = false)
                if (focus != Offset.Unspecified && (zoom != 1f || pan != Offset.Zero)) {
                    map.panZoom(pan, zoom, focus)
                    event.changes.forEach { if (it.positionChanged()) it.consume() }
                }
                val now = event.calculateCentroid(useCurrent = true)
                if (now != Offset.Unspecified) tracker.addPosition(event.changes.first().uptimeMillis, now)
            } while (event.changes.any { it.pressed })
            val v = tracker.calculateVelocity()
            if (v.x * v.x + v.y * v.y > 300f * 300f) launch { map.fling(v) }
        }
    }
}

@Composable
private fun Markers(map: MapState, clusters: List<Cluster>, markerPx: Float, onTap: (Cluster) -> Unit) {
    // Which clusters are near the screen; recomputed only when the view moved by a quarter screen.
    val quantized by remember(map) {
        derivedStateOf(structuralEqualityPolicy()) {
            val c = map.camera
            val ws = MapMath.worldSize(c.zoom)
            val q = max(1.0, max(map.size.width, map.size.height) / 4.0) / ws
            Triple(floor(c.x / q).toLong(), floor(c.y / q).toLong(), (c.zoom * 4).toInt()) to map.size
        }
    }
    val visible = remember(clusters, quantized) {
        // read without subscribing: this list changes with [quantized], not every frame
        val b = Snapshot.withoutReadObservation { map.viewBounds(marginPx = max(map.size.width, map.size.height) / 2f + markerPx) }
        clusters.filter { c ->
            val inY = c.y in b[1]..b[3]
            val inX = c.x in b[0]..b[2] || (c.x + 1) in b[0]..b[2] || (c.x - 1) in b[0]..b[2]
            inX && inY
        }.take(MAX_MARKERS)
    }
    val p = MaterialTheme.palette
    val half = (markerPx / 2).toInt()
    for (c in visible) {
        androidx.compose.runtime.key(c.coverId) {
            Box(
                Modifier
                    .offset { IntOffset(map.screenX(c.x).toInt() - half, map.screenY(c.y).toInt() - half) }
                    .size(MARKER)
                    .clip(RoundedCornerShape(EncoreRadius.small))
                    .border(2.dp, Color.White, RoundedCornerShape(EncoreRadius.small))
                    .background(p.panel2)
                    .clickable { onTap(c) },
            ) {
                AsyncImage(
                    model = Thumb(c.coverId),
                    contentDescription = null,
                    contentScale = ContentScale.Crop,
                    modifier = Modifier.fillMaxSize().padding(2.dp).clip(RoundedCornerShape(EncoreRadius.small - 2.dp)),
                )
                if (c.count > 1) {
                    Text(
                        if (c.count > 999) "999+" else c.count.toString(),
                        style = MaterialTheme.typography.labelSmall,
                        color = p.onAccent,
                        modifier = Modifier
                            .align(Alignment.BottomEnd)
                            .padding(3.dp)
                            .background(p.accent, RoundedCornerShape(EncoreRadius.pill))
                            .padding(horizontal = 5.dp),
                    )
                }
            }
        }
    }
}

@Composable
private fun PhotoStrip(loaded: Boolean, anyPhotos: Boolean, photos: List<WorldPoint>, onOpen: (Long) -> Unit) {
    val p = MaterialTheme.palette
    Column(
        Modifier
            .fillMaxWidth()
            .background(p.bg, RoundedCornerShape(topStart = EncoreRadius.card, topEnd = EncoreRadius.card))
            .navigationBarsPadding()
            .padding(top = Spacing.lg, bottom = Spacing.md),
    ) {
        val title = when {
            !loaded -> stringResource(R.string.map_loading)
            !anyPhotos -> stringResource(R.string.map_empty)
            photos.isEmpty() -> stringResource(R.string.map_none_here)
            else -> pluralStringResource(R.plurals.map_photos_here, photos.size, photos.size)
        }
        Text(title, style = MaterialTheme.typography.titleSmall, color = p.text, modifier = Modifier.padding(horizontal = Spacing.xl))
        if (photos.isNotEmpty()) {
            Spacer(Modifier.height(Spacing.md))
            LazyRow(
                contentPadding = PaddingValues(horizontal = Spacing.xl),
                horizontalArrangement = androidx.compose.foundation.layout.Arrangement.spacedBy(Spacing.xs),
                modifier = Modifier.testTag("map_strip"),
            ) {
                items(photos, key = { it.fileId }) { pt ->
                    AsyncImage(
                        model = Thumb(pt.fileId),
                        contentDescription = null,
                        contentScale = ContentScale.Crop,
                        modifier = Modifier
                            .width(76.dp)
                            .height(76.dp)
                            .clip(RoundedCornerShape(EncoreRadius.small))
                            .background(p.panel)
                            .clickable { onOpen(pt.fileId) },
                    )
                }
            }
        }
    }
}

private const val MAX_MARKERS = 120
