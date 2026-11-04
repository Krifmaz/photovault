package app.photovault.android.ui.viewer

import androidx.compose.animation.core.animate
import androidx.compose.animation.core.spring
import androidx.compose.foundation.gestures.awaitEachGesture
import androidx.compose.foundation.gestures.awaitFirstDown
import androidx.compose.foundation.gestures.calculateCentroid
import androidx.compose.foundation.gestures.calculatePan
import androidx.compose.foundation.gestures.calculateZoom
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.runtime.Stable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.isSpecified
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.input.pointer.positionChanged
import androidx.compose.ui.unit.IntSize
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.launch

/** Pinch / double-tap zoom of one viewer page. Read only in the graphics layer. */
@Stable
class ZoomState {
    var scale by mutableFloatStateOf(1f)
        private set
    var offset by mutableStateOf(Offset.Zero)
        private set

    val zoomed: Boolean get() = scale > 1.01f

    /** Zooms by [zoomChange] around [centroid] and pans by [pan], keeping the content inside the page. */
    fun transform(zoomChange: Float, pan: Offset, centroid: Offset, size: IntSize) {
        val newScale = (scale * zoomChange).coerceIn(MIN, MAX)
        val q = centroid - Offset(size.width / 2f, size.height / 2f)
        val raw = q + pan - (q - offset) * (newScale / scale)
        scale = newScale
        offset = clamp(raw, newScale, size)
    }

    /** Double tap: zoom to [DOUBLE_TAP] at [tap], or back to fit. */
    fun toggle(tap: Offset, size: IntSize, scope: CoroutineScope) {
        val fromScale = scale
        val fromOffset = offset
        val toScale = if (zoomed) 1f else DOUBLE_TAP
        val q = tap - Offset(size.width / 2f, size.height / 2f)
        val toOffset = if (toScale == 1f) Offset.Zero else clamp(q - q * toScale, toScale, size)
        scope.launch {
            animate(0f, 1f, animationSpec = spring(stiffness = 600f)) { t, _ ->
                scale = fromScale + (toScale - fromScale) * t
                offset = fromOffset + (toOffset - fromOffset) * t
            }
        }
    }

    fun reset() {
        scale = 1f
        offset = Offset.Zero
    }

    private fun clamp(o: Offset, s: Float, size: IntSize): Offset {
        val maxX = (s - 1f) * size.width / 2f
        val maxY = (s - 1f) * size.height / 2f
        return Offset(o.x.coerceIn(-maxX, maxX), o.y.coerceIn(-maxY, maxY))
    }

    companion object {
        const val MIN = 1f
        const val MAX = 6f
        const val DOUBLE_TAP = 2.5f
    }
}

/**
 * Pinch-zoom, pan while zoomed and double-tap. A single finger at fit scale is left alone, so the pager
 * (sideways) and swipe-down-to-close (downwards) get it.
 */
fun Modifier.zoomable(state: ZoomState, scope: CoroutineScope, onTap: () -> Unit): Modifier = this
    .pointerInput(state) {
        detectTapGestures(
            onTap = { onTap() },
            onDoubleTap = { state.toggle(it, size, scope) },
        )
    }
    .pointerInput(state) {
        awaitEachGesture {
            awaitFirstDown(requireUnconsumed = false)
            do {
                val event = awaitPointerEvent()
                val fingers = event.changes.count { it.pressed }
                if (fingers >= 2 || (fingers == 1 && state.zoomed)) {
                    val centroid = event.calculateCentroid(useCurrent = false)
                    if (centroid.isSpecified) state.transform(event.calculateZoom(), event.calculatePan(), centroid, size)
                    event.changes.forEach { if (it.positionChanged()) it.consume() }
                }
            } while (event.changes.any { it.pressed })
        }
    }
    .graphicsLayer {
        scaleX = state.scale
        scaleY = state.scale
        translationX = state.offset.x
        translationY = state.offset.y
    }
