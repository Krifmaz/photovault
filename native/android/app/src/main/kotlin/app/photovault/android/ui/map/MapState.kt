package app.photovault.android.ui.map

import androidx.compose.animation.core.Animatable
import androidx.compose.animation.core.FastOutSlowInEasing
import androidx.compose.animation.core.exponentialDecay
import androidx.compose.animation.core.tween
import androidx.compose.animation.core.VectorConverter
import androidx.compose.foundation.MutatePriority
import androidx.compose.foundation.MutatorMutex
import androidx.compose.runtime.Stable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.saveable.Saver
import androidx.compose.runtime.setValue
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.unit.IntSize
import androidx.compose.ui.unit.Velocity
import kotlin.math.floor
import kotlin.math.round

/**
 * The map's camera and the gestures that move it. Everything reads [camera] in the layout or draw phase, so
 * panning and zooming redraw at the display's refresh rate without recomposing.
 */
@Stable
class MapState(initial: Camera, val hasInitialView: Boolean) {
    var camera: Camera by mutableStateOf(initial.normalized())
        private set
    var size: IntSize by mutableStateOf(IntSize.Zero)
        internal set
    /** True once the camera was placed (restored, or fitted to the photos). */
    var placed: Boolean by mutableStateOf(hasInitialView)
        internal set

    private val mutex = MutatorMutex()

    private val worldSize: Double get() = MapMath.worldSize(camera.zoom)

    /** Screen position of a world point (taking the copy of the world nearest the centre). */
    fun screenX(wx: Double): Float {
        var dx = wx - camera.x
        dx -= round(dx)
        return (dx * worldSize + size.width / 2.0).toFloat()
    }

    fun screenY(wy: Double): Float = ((wy - camera.y) * worldSize + size.height / 2.0).toFloat()

    /** The visible area in world coordinates: minX, minY, maxX, maxY (x not wrapped). */
    fun viewBounds(marginPx: Float = 0f): DoubleArray {
        val ws = worldSize
        val hw = (size.width / 2.0 + marginPx) / ws
        val hh = (size.height / 2.0 + marginPx) / ws
        return doubleArrayOf(camera.x - hw, camera.y - hh, camera.x + hw, camera.y + hh)
    }

    /** One gesture step: move by [pan] pixels and scale by [zoomBy] around [focus]. */
    fun panZoom(pan: Offset, zoomBy: Float, focus: Offset) {
        val c = camera
        val ws = MapMath.worldSize(c.zoom)
        val newZoom = (c.zoom + kotlin.math.log2(zoomBy.coerceAtLeast(1e-3f))).coerceIn(MapMath.MIN_ZOOM, MapMath.MAX_ZOOM)
        val ws2 = MapMath.worldSize(newZoom)
        val fx = focus.x - size.width / 2.0
        val fy = focus.y - size.height / 2.0
        // the world point under the focus stays under it
        val wx = c.x + fx / ws
        val wy = c.y + fy / ws
        camera = Camera(wx - (fx + pan.x) / ws2, wy - (fy + pan.y) / ws2, newZoom).normalized()
    }

    fun place(c: Camera) {
        camera = c.normalized()
        placed = true
    }

    /** Stops whatever animation runs (a finger touched the map). */
    suspend fun stop() = mutex.mutate(MutatePriority.UserInput) { }

    suspend fun fling(velocity: Velocity) = mutex.mutate(MutatePriority.Default) {
        val anim = Animatable(Offset.Zero, Offset.VectorConverter)
        var last = Offset.Zero
        anim.animateDecay(Offset(velocity.x, velocity.y), exponentialDecay<Offset>(frictionMultiplier = 1.6f)) {
            val d = value - last
            last = value
            panZoom(d, 1f, Offset(size.width / 2f, size.height / 2f))
        }
    }

    /** Animates to [target]: the path goes straight in world space while the zoom eases. */
    suspend fun animateTo(target: Camera, durationMs: Int = 450) = mutex.mutate(MutatePriority.Default) {
        val from = camera
        var tx = target.x
        // the short way round the antimeridian
        val dx = tx - from.x
        tx = from.x + (dx - round(dx))
        val t = Animatable(0f)
        t.animateTo(1f, tween(durationMs, easing = FastOutSlowInEasing)) {
            val f = value.toDouble()
            camera = Camera(from.x + (tx - from.x) * f, from.y + (target.y - from.y) * f, from.zoom + (target.zoom - from.zoom) * value).normalized()
        }
    }

    suspend fun zoomBy(delta: Float, focus: Offset) {
        val c = camera
        val ws = MapMath.worldSize(c.zoom)
        val wx = c.x + (focus.x - size.width / 2.0) / ws
        val wy = c.y + (focus.y - size.height / 2.0) / ws
        val z = (c.zoom + delta).coerceIn(MapMath.MIN_ZOOM, MapMath.MAX_ZOOM)
        val ws2 = MapMath.worldSize(z)
        animateTo(Camera(wx - (focus.x - size.width / 2.0) / ws2, wy - (focus.y - size.height / 2.0) / ws2, z), 250)
    }

    companion object {
        val Saver: Saver<MapState, DoubleArray> = Saver(
            save = { s -> if (s.placed) doubleArrayOf(s.camera.x, s.camera.y, s.camera.zoom.toDouble()) else doubleArrayOf() },
            restore = { a -> if (a.size == 3) MapState(Camera(a[0], a[1], a[2].toFloat()), true) else MapState(WORLD, false) },
        )
        val WORLD = Camera(0.5, 0.42, 1.5f)

        /** Integer zoom the clusters are computed at for a camera zoom. */
        fun clusterZoom(zoom: Float): Int = floor(zoom.toDouble()).toInt()
    }
}
