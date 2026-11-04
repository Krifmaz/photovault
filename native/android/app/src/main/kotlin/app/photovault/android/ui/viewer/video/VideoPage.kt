package app.photovault.android.ui.viewer.video

import androidx.annotation.OptIn
import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.core.tween
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawing
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Slider
import androidx.compose.material3.SliderDefaults
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.media3.common.util.UnstableApi
import androidx.media3.ui.compose.PlayerSurface
import androidx.media3.ui.compose.SURFACE_TYPE_TEXTURE_VIEW
import app.photovault.android.R
import app.photovault.android.data.TimelineItem
import app.photovault.android.ui.theme.EncoreMotion
import app.photovault.android.ui.theme.Spacing
import app.photovault.android.ui.theme.palette
import java.util.Locale

/**
 * A video page of the viewer. Only the settled page ([active]) attaches the shared player's surface; the others
 * show the thumbnail (drawn underneath by the page) with a play badge, so swiping never moves a live surface
 * between pages. The surface is a TextureView so it follows the pager's and the dismiss gesture's transforms.
 */
@Composable
fun VideoPage(item: TimelineItem, controller: VideoPlaybackController, active: Boolean) {
    LaunchedEffect(active, item.id) {
        if (active) controller.open(item) else controller.leave(item.id)
    }
    DisposableEffect(item.id) {
        onDispose { controller.leave(item.id) }
    }
    if (!active) {
        Box(Modifier.fillMaxSize()) {
            PlayBadge(Modifier.align(Alignment.Center))
            if (item.hasStream) StreamBadge(Modifier.align(Alignment.TopEnd).windowInsetsPadding(WindowInsets.safeDrawing).padding(top = 56.dp, end = Spacing.lg))
        }
        return
    }
    ActiveVideo(controller)
}

@OptIn(UnstableApi::class)
@Composable
private fun ActiveVideo(controller: VideoPlaybackController) {
    val state by controller.state.collectAsStateWithLifecycle()
    val s = state
    Box(Modifier.fillMaxSize().testTag("video_player")) {
        val engine = controller.engine as? ExoPlaybackEngine
        val showSurface = s.phase == VideoPhase.Buffering || s.phase == VideoPhase.Ready || s.phase == VideoPhase.Ended
        if (showSurface && engine != null) {
            val ratio = if (s.videoWidth > 0 && s.videoHeight > 0) s.videoWidth.toFloat() / s.videoHeight else 16f / 9f
            PlayerSurface(
                player = engine.player,
                surfaceType = SURFACE_TYPE_TEXTURE_VIEW,
                modifier = Modifier
                    .align(Alignment.Center)
                    .aspectRatio(ratio, matchHeightConstraintsFirst = ratio < 1f)
                    // Hidden until the first frame, so the thumbnail underneath shows instead of a black box.
                    .graphicsLayer { alpha = if (state.firstFrame) 1f else 0f },
            )
        }
        when {
            s.phase == VideoPhase.Error -> Text(
                stringResource(R.string.viewer_video_error, s.error.orEmpty()),
                color = Color.White,
                style = MaterialTheme.typography.bodyMedium,
                modifier = Modifier
                    .align(Alignment.Center)
                    .clip(MaterialTheme.shapes.small)
                    .background(Color.Black.copy(alpha = 0.6f))
                    .padding(Spacing.lg),
            )
            s.phase == VideoPhase.Resolving || (s.phase == VideoPhase.Buffering && !s.controlsVisible) ||
                (s.phase == VideoPhase.Buffering && !s.firstFrame) ->
                CircularProgressIndicator(color = MaterialTheme.palette.accent2, modifier = Modifier.align(Alignment.Center))
        }
        AnimatedVisibility(
            visible = s.controlsVisible && s.phase != VideoPhase.Resolving && s.phase != VideoPhase.Error,
            enter = fadeIn(tween(EncoreMotion.NORMAL_MS)),
            exit = fadeOut(tween(EncoreMotion.NORMAL_MS)),
            modifier = Modifier.fillMaxSize(),
        ) {
            Controls(controller, s)
        }
    }
}

@Composable
private fun Controls(controller: VideoPlaybackController, s: VideoUiState) {
    Box(Modifier.fillMaxSize()) {
        if (!(s.phase == VideoPhase.Buffering && !s.firstFrame)) {
            val playLabel = stringResource(if (s.playing) R.string.video_pause else R.string.video_play)
            IconButton(
                onClick = controller::togglePlay,
                modifier = Modifier
                    .align(Alignment.Center)
                    .size(64.dp)
                    .clip(CircleShape)
                    .background(Color.Black.copy(alpha = 0.45f))
                    .semantics { contentDescription = playLabel },
            ) {
                Icon(
                    painterResource(if (s.playing) R.drawable.ic_pause else R.drawable.ic_play),
                    contentDescription = null,
                    tint = Color.White,
                    modifier = Modifier.size(36.dp),
                )
            }
        }
        Column(
            Modifier
                .align(Alignment.BottomCenter)
                .fillMaxWidth()
                .background(Brush.verticalGradient(listOf(Color.Transparent, Color.Black.copy(alpha = 0.6f))))
                .windowInsetsPadding(WindowInsets.safeDrawing)
                .padding(horizontal = Spacing.lg, vertical = Spacing.xs),
        ) {
            Row(
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(Spacing.md),
            ) {
                if (s.isStream) StreamBadge()
                SeekBar(controller, Modifier.weight(1f))
                IconButton(onClick = controller::toggleMute) {
                    Icon(
                        painterResource(if (s.muted) R.drawable.ic_volume_off else R.drawable.ic_volume_up),
                        contentDescription = stringResource(if (s.muted) R.string.video_unmute else R.string.video_mute),
                        tint = Color.White,
                    )
                }
            }
        }
    }
}

/** Position and seek: the only composable that reads the ticking [VideoPlaybackController.progress]. */
@Composable
private fun SeekBar(controller: VideoPlaybackController, modifier: Modifier = Modifier) {
    val progress by controller.progress.collectAsStateWithLifecycle()
    var dragging by remember { mutableStateOf(false) }
    var dragValue by remember { mutableFloatStateOf(0f) }
    val duration = progress.durationMs.coerceAtLeast(0)
    val shown = if (dragging) (dragValue * duration).toLong() else progress.positionMs
    val label = stringResource(R.string.video_seek)
    Row(modifier, verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(Spacing.sm)) {
        Text(formatTime(shown), color = Color.White, style = MaterialTheme.typography.labelMedium)
        Slider(
            value = if (dragging) dragValue else if (duration > 0) (progress.positionMs.toFloat() / duration).coerceIn(0f, 1f) else 0f,
            onValueChange = {
                dragging = true
                dragValue = it
            },
            onValueChangeFinished = {
                if (duration > 0) controller.seekTo((dragValue * duration).toLong())
                dragging = false
            },
            enabled = duration > 0,
            colors = SliderDefaults.colors(
                thumbColor = Color.White,
                activeTrackColor = MaterialTheme.palette.accent2,
                inactiveTrackColor = Color.White.copy(alpha = 0.3f),
            ),
            modifier = Modifier.weight(1f).semantics { contentDescription = label },
        )
        Text(formatTime(duration), color = Color.White.copy(alpha = 0.8f), style = MaterialTheme.typography.labelMedium)
    }
}

@Composable
private fun PlayBadge(modifier: Modifier = Modifier) {
    Box(
        modifier
            .size(64.dp)
            .clip(CircleShape)
            .background(Color.Black.copy(alpha = 0.5f)),
        contentAlignment = Alignment.Center,
    ) {
        Icon(painterResource(R.drawable.ic_play), contentDescription = null, tint = Color.White, modifier = Modifier.size(32.dp))
    }
}

/** The small badge of videos that play from a streamable version. */
@Composable
fun StreamBadge(modifier: Modifier = Modifier) {
    Text(
        stringResource(R.string.video_stream_badge),
        color = Color.White,
        style = MaterialTheme.typography.labelSmall,
        modifier = modifier
            .clip(MaterialTheme.shapes.small)
            .background(Color.Black.copy(alpha = 0.5f))
            .padding(horizontal = Spacing.sm, vertical = Spacing.xxs),
    )
}

/** `m:ss`, or `h:mm:ss` from an hour. */
internal fun formatTime(ms: Long): String {
    val total = (ms.coerceAtLeast(0) / 1000)
    val h = total / 3600
    val m = (total % 3600) / 60
    val sec = total % 60
    return if (h > 0) String.format(Locale.ROOT, "%d:%02d:%02d", h, m, sec) else String.format(Locale.ROOT, "%d:%02d", m, sec)
}
