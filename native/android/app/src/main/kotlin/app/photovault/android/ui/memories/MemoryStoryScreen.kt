package app.photovault.android.ui.memories

import androidx.activity.compose.BackHandler
import androidx.compose.animation.core.Animatable
import androidx.compose.animation.core.LinearEasing
import androidx.compose.animation.core.tween
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.gestures.awaitEachGesture
import androidx.compose.foundation.gestures.awaitFirstDown
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawing
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.produceState
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.geometry.CornerRadius
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.input.pointer.positionChange
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.compose.LocalLifecycleOwner
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.compose.currentStateAsState
import app.photovault.android.R
import app.photovault.android.data.CoreError
import app.photovault.android.data.FileKind
import app.photovault.android.data.TimelineItem
import app.photovault.android.ui.image.Thumb
import app.photovault.android.ui.nav.sharedPhoto
import app.photovault.android.ui.theme.Spacing
import coil3.SingletonImageLoader
import coil3.compose.AsyncImage
import coil3.request.ImageRequest
import coil3.request.crossfade
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.launch
import java.io.File
import java.io.IOException
import java.time.Instant
import java.time.ZoneId
import java.time.format.DateTimeFormatter
import java.time.format.FormatStyle
import kotlin.math.abs
import kotlin.math.min
import kotlin.math.roundToInt

/** How long one photo stays up (Ente: memory_progress_indicator.dart, 5 s). */
private const val PAGE_MS = 5_000

/**
 * A memory as a full-screen story (Ente's full-screen memory): progress segments on top, tap the left third
 * for the previous photo and elsewhere for the next, hold to pause, swipe down to close, "Open" shows the
 * photo in the regular viewer. The progress is a plain linear animation read in the draw phase, so the
 * running timer never recomposes the screen.
 *
 * Based on Ente (ente.com): mobile/apps/photos/lib/ui/home/memories/full_screen_memory.dart.
 */
@Composable
fun MemoryStoryScreen(
    vm: MemoryStoryViewModel,
    returnIndex: Flow<Int?>,
    onReturnConsumed: () -> Unit,
    onClose: () -> Unit,
    onOpenPhoto: (memoryId: String, index: Int, fileId: Long) -> Unit,
) {
    val state by vm.state.collectAsStateWithLifecycle()
    val close by rememberUpdatedState(onClose)
    BackHandler { close() }

    LaunchedEffect(returnIndex) {
        returnIndex.collect { index ->
            if (index == null) return@collect
            onReturnConsumed()
            vm.jumpTo(index)
        }
    }
    LaunchedEffect(state.missing) {
        if (state.missing) close()
    }

    val memory = state.memory
    if (memory == null) {
        Box(Modifier.fillMaxSize().background(Color.Black), contentAlignment = Alignment.Center) {
            if (!state.missing) CircularProgressIndicator(color = Color.White)
        }
        return
    }
    val item = memory.items.getOrNull(state.index) ?: return

    val scope = rememberCoroutineScope()
    val context = LocalContext.current
    val progress = remember { Animatable(0f) }
    val dismiss = remember { Animatable(0f) }
    var held by remember { mutableStateOf(false) }
    var shownIndex by remember { mutableIntStateOf(-1) }
    val lifecycle by LocalLifecycleOwner.current.lifecycle.currentStateAsState()
    val paused = held || !lifecycle.isAtLeast(Lifecycle.State.RESUMED)

    // The timer: restarts from 0 on a new photo, keeps its value while paused (the effect is cancelled).
    LaunchedEffect(memory.id, state.index, paused) {
        if (shownIndex != state.index) {
            progress.snapTo(0f)
            shownIndex = state.index
        }
        if (paused) return@LaunchedEffect
        val remaining = ((1f - progress.value) * PAGE_MS).roundToInt().coerceAtLeast(0)
        progress.animateTo(1f, tween(durationMillis = remaining, easing = LinearEasing))
        if (!vm.next()) close()
    }

    // Warm the next photo: its thumbnail into Coil's memory cache, its original into the story's cache.
    LaunchedEffect(memory.id, state.index) {
        val next = memory.items.getOrNull(state.index + 1) ?: return@LaunchedEffect
        SingletonImageLoader.get(context).enqueue(ImageRequest.Builder(context).data(Thumb(next.id)).build())
        try {
            vm.original(next)
        } catch (e: CoreError) {
            // shown from its thumbnail
        } catch (e: IOException) {
            // shown from its thumbnail
        }
    }

    Box(
        Modifier
            .fillMaxSize()
            .testTag("memory_story")
            .drawBehind {
                // fades out while dragged down; read in the draw phase only
                val fraction = min(abs(dismiss.value) / (size.height / 2.5f), 0.85f)
                drawRect(Color.Black.copy(alpha = 1f - fraction))
            },
    ) {
        Box(
            Modifier
                .fillMaxSize()
                .graphicsLayer {
                    val d = dismiss.value
                    translationY = d
                    val s = 1f - min(abs(d) / size.height, 0.25f)
                    scaleX = s
                    scaleY = s
                },
        ) {
            StoryPage(item, vm)
        }

        // Gestures, under the chrome so the buttons get their own taps.
        Box(
            Modifier
                .fillMaxSize()
                .pointerInput(vm) {
                    awaitEachGesture {
                        val down = awaitFirstDown(requireUnconsumed = false)
                        held = true
                        var dragY = 0f
                        var dragging = false
                        var released = false
                        var upTime = down.uptimeMillis
                        while (true) {
                            val event = awaitPointerEvent()
                            val change = event.changes.firstOrNull { it.id == down.id } ?: break
                            if (!change.pressed) {
                                released = true
                                upTime = change.uptimeMillis
                                break
                            }
                            dragY += change.positionChange().y
                            if (!dragging && dragY > viewConfiguration.touchSlop) dragging = true
                            if (dragging) {
                                change.consume()
                                val y = (dragY - viewConfiguration.touchSlop).coerceAtLeast(0f)
                                scope.launch { dismiss.snapTo(y) }
                            }
                        }
                        held = false
                        when {
                            dragging -> {
                                if (dismiss.value > size.height / 6f) close() else scope.launch { dismiss.animateTo(0f) }
                            }
                            released && abs(dragY) <= viewConfiguration.touchSlop &&
                                upTime - down.uptimeMillis < viewConfiguration.longPressTimeoutMillis -> {
                                if (down.position.x < size.width / 3f) {
                                    vm.previous()
                                } else if (!vm.next()) {
                                    close()
                                }
                            }
                            // a hold: just resumes
                        }
                    }
                },
        )

        StoryChrome(
            title = memory.title,
            subtitle = memory.subtitle,
            item = item,
            count = memory.items.size,
            index = state.index,
            progress = { progress.value },
            onClose = { close() },
            onOpen = { onOpenPhoto(memory.id, state.index, item.id) },
        )
    }
}

@Composable
private fun StoryPage(item: TimelineItem, vm: MemoryStoryViewModel) {
    Box(Modifier.fillMaxSize().sharedPhoto(item.id), contentAlignment = Alignment.Center) {
        AsyncImage(
            model = Thumb(item.id),
            contentDescription = null,
            contentScale = ContentScale.Fit,
            modifier = Modifier.fillMaxSize(),
        )
        if (item.kind == FileKind.Video) {
            Box(
                Modifier
                    .size(64.dp)
                    .clip(CircleShape)
                    .background(Color.Black.copy(alpha = 0.5f)),
                contentAlignment = Alignment.Center,
            ) {
                Icon(painterResource(R.drawable.ic_play), contentDescription = null, tint = Color.White, modifier = Modifier.size(32.dp))
            }
        } else {
            StoryOriginal(item, vm)
        }
    }
}

/** The decrypted original over the thumbnail, decoded at screen size (a story does not zoom). */
@Composable
private fun StoryOriginal(item: TimelineItem, vm: MemoryStoryViewModel) {
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
    val request = remember(f) {
        ImageRequest.Builder(context)
            .data(f)
            .memoryCacheKey("memory-original:${item.id}")
            .crossfade(true)
            .build()
    }
    AsyncImage(model = request, contentDescription = item.title, contentScale = ContentScale.Fit, modifier = Modifier.fillMaxSize())
}

@Composable
private fun StoryChrome(
    title: String,
    subtitle: String?,
    item: TimelineItem,
    count: Int,
    index: Int,
    progress: () -> Float,
    onClose: () -> Unit,
    onOpen: () -> Unit,
) {
    val formatter = remember { DateTimeFormatter.ofLocalizedDate(FormatStyle.LONG) }
    Box(Modifier.fillMaxSize()) {
        Column(
            Modifier
                .fillMaxWidth()
                .background(Brush.verticalGradient(listOf(Color.Black.copy(alpha = 0.55f), Color.Transparent)))
                .windowInsetsPadding(WindowInsets.safeDrawing)
                .padding(horizontal = Spacing.lg, vertical = Spacing.md),
            verticalArrangement = Arrangement.spacedBy(Spacing.md),
        ) {
            ProgressSegments(count, index, progress)
            Row(verticalAlignment = Alignment.CenterVertically) {
                Column(Modifier.weight(1f)) {
                    Text(
                        title,
                        color = Color.White,
                        style = MaterialTheme.typography.titleMedium,
                        maxLines = 2,
                        overflow = TextOverflow.Ellipsis,
                    )
                    val date = remember(item.takenAtUs) {
                        formatter.format(Instant.ofEpochMilli(Math.floorDiv(item.takenAtUs, 1000L)).atZone(ZoneId.systemDefault()))
                    }
                    Text(
                        subtitle ?: date,
                        color = Color.White.copy(alpha = 0.75f),
                        style = MaterialTheme.typography.bodySmall,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                    )
                }
                IconButton(onClick = onClose, modifier = Modifier.testTag("memory_close")) {
                    Icon(painterResource(R.drawable.ic_memory_close), contentDescription = stringResource(R.string.memory_close), tint = Color.White)
                }
            }
        }
        TextButton(
            onClick = onOpen,
            modifier = Modifier
                .align(Alignment.BottomCenter)
                .windowInsetsPadding(WindowInsets.safeDrawing)
                .padding(bottom = Spacing.xl)
                .testTag("memory_open_photo"),
        ) {
            Text(stringResource(R.string.memory_open_photo), color = Color.White, style = MaterialTheme.typography.labelLarge)
        }
    }
}

/** One segment per photo: done ones full, the current one filling linearly, upcoming ones faint. */
@Composable
private fun ProgressSegments(count: Int, index: Int, progress: () -> Float) {
    Canvas(
        Modifier
            .fillMaxWidth()
            .height(3.dp),
    ) {
        if (count <= 0) return@Canvas
        val gap = if (size.width / count > 6.dp.toPx()) 3.dp.toPx() else 0f
        val w = (size.width - gap * (count - 1)) / count
        val h = size.height
        val r = CornerRadius(h / 2f, h / 2f)
        val track = Color.White.copy(alpha = 0.3f)
        for (i in 0 until count) {
            val x = i * (w + gap)
            drawRoundRect(track, topLeft = Offset(x, 0f), size = Size(w, h), cornerRadius = r)
            val fill = when {
                i < index -> 1f
                i == index -> progress().coerceIn(0f, 1f)
                else -> 0f
            }
            if (fill > 0f) drawRoundRect(Color.White, topLeft = Offset(x, 0f), size = Size(w * fill, h), cornerRadius = r)
        }
    }
}
