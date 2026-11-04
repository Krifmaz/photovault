package app.photovault.android.ui.viewer

import android.content.Context
import androidx.activity.compose.BackHandler
import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.core.Animatable
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.foundation.background
import androidx.compose.foundation.gestures.detectVerticalDragGestures
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawing
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.foundation.pager.HorizontalPager
import androidx.compose.foundation.pager.PagerState
import androidx.compose.foundation.pager.rememberPagerState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.Stable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.produceState
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.runtime.snapshotFlow
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.viewinterop.AndroidView
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.media3.common.MediaItem
import androidx.media3.common.MimeTypes
import androidx.media3.common.Player
import androidx.media3.exoplayer.ExoPlayer
import androidx.media3.ui.PlayerView
import app.photovault.android.R
import app.photovault.android.data.CoreError
import app.photovault.android.data.FileKind
import app.photovault.android.data.TimelineItem
import app.photovault.android.ui.image.Thumb
import app.photovault.android.ui.nav.sharedPhoto
import app.photovault.android.ui.theme.Spacing
import app.photovault.android.ui.theme.palette
import coil3.compose.AsyncImage
import coil3.request.ImageRequest
import coil3.request.crossfade
import kotlinx.coroutines.launch
import java.io.File
import java.io.IOException
import java.time.Instant
import java.time.ZoneId
import java.time.format.DateTimeFormatter
import java.time.format.FormatStyle
import kotlin.math.abs
import kotlin.math.min

/** Full-screen photos and videos: swipe between them, pinch/double-tap to zoom, swipe down to close. */
@Composable
fun ViewerScreen(vm: ViewerViewModel, startIndex: Int, onClose: (Int) -> Unit) {
    val items by vm.items.collectAsStateWithLifecycle()
    val loaded by vm.loaded.collectAsStateWithLifecycle()
    val total by vm.total.collectAsStateWithLifecycle()
    // Restored after process death, the view model loads the timeline first (up to the opened photo), so
    // the pager starts on the right page.
    val ready = loaded && (items.size > startIndex || items.size.toLong() >= total)
    if (!ready || items.isEmpty()) {
        Box(Modifier.fillMaxSize().background(Color.Black), contentAlignment = Alignment.Center) {
            if (!ready) CircularProgressIndicator(color = Color.White)
        }
        return
    }
    val pager = rememberPagerState(initialPage = startIndex.coerceIn(0, items.lastIndex)) { items.size }
    val scope = rememberCoroutineScope()
    val dismiss = remember { Animatable(0f) }
    var chrome by remember { mutableStateOf(true) }
    var zoomed by remember { mutableStateOf(false) }
    val player = rememberViewerPlayer()
    val close = { onClose(pager.currentPage) }

    BackHandler(onBack = close)
    LaunchedEffect(pager) {
        snapshotFlow { pager.settledPage }.collect { vm.onPage(it) }
    }

    Box(
        Modifier
            .fillMaxSize()
            .drawBehind {
                // fades out while the photo is dragged down; read in the draw phase only
                val fraction = min(abs(dismiss.value) / (size.height / 2.5f), 0.85f)
                drawRect(Color.Black.copy(alpha = 1f - fraction))
            }
            .pointerInput(Unit) {
                detectVerticalDragGestures(
                    onDragEnd = {
                        if (abs(dismiss.value) > size.height / 6f) close()
                        else scope.launch { dismiss.animateTo(0f) }
                    },
                    onDragCancel = { scope.launch { dismiss.animateTo(0f) } },
                    onVerticalDrag = { change, dy ->
                        if (!zoomed) {
                            change.consume()
                            scope.launch { dismiss.snapTo(dismiss.value + dy) }
                        }
                    },
                )
            },
    ) {
        HorizontalPager(
            state = pager,
            key = { index -> items.getOrNull(index)?.id ?: index },
            userScrollEnabled = !zoomed,
            beyondViewportPageCount = 1,
            pageSpacing = 12.dp,
            modifier = Modifier
                .fillMaxSize()
                .testTag("viewer_pager")
                .graphicsLayer {
                    val d = dismiss.value
                    translationY = d
                    val s = 1f - min(abs(d) / size.height, 0.25f)
                    scaleX = s
                    scaleY = s
                },
        ) { page ->
            val item = items.getOrNull(page) ?: return@HorizontalPager
            ViewerPage(
                item = item,
                vm = vm,
                pager = pager,
                page = page,
                player = player,
                onTap = { chrome = !chrome },
                onZoomed = { if (page == pager.currentPage) zoomed = it },
            )
        }
        AnimatedVisibility(visible = chrome && !zoomed, enter = fadeIn(), exit = fadeOut()) {
            TopChrome(items.getOrNull(pager.currentPage), close)
        }
    }
}

@Composable
private fun ViewerPage(
    item: TimelineItem,
    vm: ViewerViewModel,
    pager: PagerState,
    page: Int,
    player: ViewerPlayer,
    onTap: () -> Unit,
    onZoomed: (Boolean) -> Unit,
) {
    val zoom = remember { ZoomState() }
    val scope = rememberCoroutineScope()
    val isCurrent by remember(pager, page) { androidx.compose.runtime.derivedStateOf { pager.settledPage == page } }
    val sharedHere = remember(pager, page) { page == pager.currentPage }

    LaunchedEffect(zoom) {
        snapshotFlow { zoom.zoomed }.collect { onZoomed(it) }
    }
    LaunchedEffect(isCurrent) {
        if (!isCurrent) zoom.reset()
    }

    BoxWithConstraints(Modifier.fillMaxSize()) {
        val maxW = if (constraints.hasBoundedWidth) constraints.maxWidth else 2048
        val maxH = if (constraints.hasBoundedHeight) constraints.maxHeight else 2048
        Box(
            Modifier
                .fillMaxSize()
                .zoomable(zoom, scope, onTap),
        ) {
            Box(Modifier.fillMaxSize().then(if (sharedHere) Modifier.sharedPhoto(item.id) else Modifier)) {
                AsyncImage(
                    model = Thumb(item.id),
                    contentDescription = null,
                    contentScale = ContentScale.Fit,
                    modifier = Modifier.fillMaxSize(),
                )
                when (item.kind) {
                    FileKind.Video -> VideoContent(item, vm, player, active = isCurrent)
                    else -> OriginalImage(item, vm, maxW, maxH)
                }
            }
        }
    }
}

/** The decrypted original, decoded at up to twice the page size so zooming stays sharp. */
@Composable
private fun OriginalImage(item: TimelineItem, vm: ViewerViewModel, maxW: Int, maxH: Int) {
    val context = LocalContext.current
    val file by produceState<File?>(null, item.id) {
        value = try {
            vm.original(item)
        } catch (e: CoreError) {
            null
        } catch (e: IOException) {
            null
        }
    }
    val f = file ?: return
    val request = remember(f, maxW, maxH) {
        ImageRequest.Builder(context)
            .data(f)
            .memoryCacheKey("original:${item.id}")
            .size((maxW * 2).coerceAtMost(4096), (maxH * 2).coerceAtMost(4096))
            .crossfade(true)
            .build()
    }
    AsyncImage(model = request, contentDescription = item.title, contentScale = ContentScale.Fit, modifier = Modifier.fillMaxSize())
}

@Composable
private fun VideoContent(item: TimelineItem, vm: ViewerViewModel, player: ViewerPlayer, active: Boolean) {
    if (!active) {
        PlayBadge()
        return
    }
    val source by produceState<Result<VideoSource>?>(null, item.id) {
        value = try {
            Result.success(vm.video(item))
        } catch (e: CoreError) {
            Result.failure(e)
        } catch (e: IOException) {
            Result.failure(e)
        }
    }
    val result = source
    when {
        result == null -> Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
            CircularProgressIndicator(color = MaterialTheme.palette.accent2)
        }
        result.isFailure -> Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
            Text(
                stringResource(R.string.viewer_video_error, result.exceptionOrNull()?.message.orEmpty()),
                color = Color.White,
                style = MaterialTheme.typography.bodyMedium,
                modifier = Modifier
                    .clip(MaterialTheme.shapes.small)
                    .background(Color.Black.copy(alpha = 0.6f))
                    .padding(Spacing.lg),
            )
        }
        else -> {
            val src = result.getOrThrow()
            val exo = player.get()
            DisposableEffect(src) {
                val media = MediaItem.Builder().setUri(src.uri).apply {
                    if (src.isHls) setMimeType(MimeTypes.APPLICATION_M3U8)
                }.build()
                exo.setMediaItem(media)
                exo.repeatMode = Player.REPEAT_MODE_OFF
                exo.prepare()
                exo.playWhenReady = true
                onDispose {
                    exo.stop()
                    exo.clearMediaItems()
                }
            }
            AndroidView(
                factory = { ctx -> PlayerView(ctx).apply { useController = true } },
                update = { view -> view.player = exo },
                onRelease = { view -> view.player = null },
                modifier = Modifier.fillMaxSize(),
            )
        }
    }
}

@Composable
private fun PlayBadge() {
    Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
        Box(
            Modifier
                .size(64.dp)
                .clip(CircleShape)
                .background(Color.Black.copy(alpha = 0.5f)),
            contentAlignment = Alignment.Center,
        ) {
            Icon(painterResource(R.drawable.ic_play), contentDescription = null, tint = Color.White, modifier = Modifier.size(32.dp))
        }
    }
}

@Composable
private fun TopChrome(item: TimelineItem?, onClose: () -> Unit) {
    val formatter = remember { DateTimeFormatter.ofLocalizedDateTime(FormatStyle.MEDIUM, FormatStyle.SHORT) }
    Row(
        Modifier
            .fillMaxWidth()
            .background(Brush.verticalGradient(listOf(Color.Black.copy(alpha = 0.6f), Color.Transparent)))
            .windowInsetsPadding(WindowInsets.safeDrawing)
            .padding(horizontal = Spacing.xs, vertical = Spacing.xs),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(Spacing.xs),
    ) {
        IconButton(onClick = onClose) {
            Icon(painterResource(R.drawable.ic_arrow_back), contentDescription = stringResource(R.string.back), tint = Color.White)
        }
        if (item != null) {
            Column(Modifier.weight(1f)) {
                val date = remember(item.takenAtUs) {
                    formatter.format(Instant.ofEpochMilli(item.takenAtUs / 1000).atZone(ZoneId.systemDefault()))
                }
                Text(date, color = Color.White, style = MaterialTheme.typography.titleMedium, maxLines = 1)
                Text(
                    item.title,
                    color = Color.White.copy(alpha = 0.7f),
                    style = MaterialTheme.typography.bodySmall,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
            }
        }
    }
}

/** One ExoPlayer for the viewer, created on the first video and released with the screen. */
@Stable
class ViewerPlayer(private val context: Context) {
    private var player: ExoPlayer? = null

    fun get(): ExoPlayer = player ?: ExoPlayer.Builder(context).build().also { player = it }

    fun release() {
        player?.release()
        player = null
    }
}

@Composable
private fun rememberViewerPlayer(): ViewerPlayer {
    val context = LocalContext.current
    val holder = remember { ViewerPlayer(context.applicationContext) }
    DisposableEffect(holder) {
        onDispose { holder.release() }
    }
    return holder
}
