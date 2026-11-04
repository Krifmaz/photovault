package app.photovault.android.ui.map

import kotlin.math.PI
import kotlin.math.atan
import kotlin.math.exp
import kotlin.math.floor
import kotlin.math.ln
import kotlin.math.log2
import kotlin.math.max
import kotlin.math.min
import kotlin.math.pow
import kotlin.math.sin

/**
 * Web Mercator (EPSG:3857), the projection of OpenStreetMap's tiles. "World" coordinates are normalised to
 * 0..1 on both axes (x east, y south), so a position at zoom z is `world * TILE * 2^z` pixels.
 */
object MapMath {
    const val TILE = 256
    const val MIN_ZOOM = 1f
    /** tile.openstreetmap.org serves zoom 0..19. */
    const val MAX_TILE_ZOOM = 19
    const val MAX_ZOOM = 19f
    private const val MAX_LAT = 85.05112878

    fun worldX(lon: Double): Double = (lon.coerceIn(-180.0, 180.0) + 180.0) / 360.0

    fun worldY(lat: Double): Double {
        val s = sin(lat.coerceIn(-MAX_LAT, MAX_LAT) * PI / 180.0)
        return (0.5 - ln((1 + s) / (1 - s)) / (4 * PI)).coerceIn(0.0, 1.0)
    }

    fun lon(worldX: Double): Double = worldX * 360.0 - 180.0

    fun lat(worldY: Double): Double = (2 * atan(exp((0.5 - worldY) * 2 * PI)) - PI / 2) * 180.0 / PI

    /** Size of the whole world in pixels at [zoom]. */
    fun worldSize(zoom: Float): Double = TILE * 2.0.pow(zoom.toDouble())

    /**
     * The camera that shows every point of a bounding box (world coordinates) in a [width] x [height] pixel
     * view with [padding] pixels free on each side, never closer than [maxZoom].
     */
    fun fit(minX: Double, minY: Double, maxX: Double, maxY: Double, width: Float, height: Float, padding: Float, maxZoom: Float = 16f): Camera {
        val w = max(1f, width - 2 * padding)
        val h = max(1f, height - 2 * padding)
        val spanX = max(maxX - minX, 1e-9)
        val spanY = max(maxY - minY, 1e-9)
        val zx = log2(w / (spanX * TILE))
        val zy = log2(h / (spanY * TILE))
        val zoom = min(zx, zy).toFloat().coerceIn(MIN_ZOOM, maxZoom)
        return Camera((minX + maxX) / 2, (minY + maxY) / 2, zoom)
    }

    /** The integer tile zoom drawn for a camera zoom (tiles are then scaled by up to ±50 %). */
    fun tileZoom(zoom: Float): Int = floor(zoom.toDouble() + 0.5).toInt().coerceIn(0, MAX_TILE_ZOOM)
}

/** Where the map looks: the centre in world coordinates and a fractional zoom. */
data class Camera(val x: Double, val y: Double, val zoom: Float) {
    /** Keeps the centre inside the world (x wraps, y clamps) and the zoom in range. */
    fun normalized(): Camera {
        val nx = x - floor(x)
        return Camera(nx, y.coerceIn(0.0, 1.0), zoom.coerceIn(MapMath.MIN_ZOOM, MapMath.MAX_ZOOM))
    }
}

/** A point projected once, so panning and zooming only multiply. */
data class WorldPoint(val fileId: Long, val x: Double, val y: Double, val takenAtUs: Long)

/** A group of nearby photos at one zoom level: drawn as the newest photo with a count. */
data class Cluster(
    val coverId: Long,
    val x: Double,
    val y: Double,
    val count: Int,
    val minX: Double,
    val minY: Double,
    val maxX: Double,
    val maxY: Double,
) {
    /** Every photo sits on (nearly) the same spot: zooming in will not split it. */
    val isSpot: Boolean get() = (maxX - minX) < 1e-7 && (maxY - minY) < 1e-7
}

object Clustering {
    /**
     * Grid clustering at integer [zoom]: points within the same [cellPx] square share a cluster, placed at
     * their mean. [points] must be newest first, so each cluster's cover is its newest photo. O(n).
     */
    fun cluster(points: List<WorldPoint>, zoom: Int, cellPx: Float): List<Cluster> {
        if (points.isEmpty()) return emptyList()
        val cells = MapMath.worldSize(zoom.toFloat()) / cellPx
        val acc = HashMap<Long, Acc>(min(points.size, 4096))
        for (p in points) {
            val cx = floor(p.x * cells).toLong()
            val cy = floor(p.y * cells).toLong()
            val key = (cx shl 32) xor (cy and 0xffffffffL)
            val a = acc.getOrPut(key) { Acc(p.fileId) }
            a.add(p)
        }
        return acc.values.map { it.toCluster() }.sortedByDescending { it.count }
    }

    /** Points inside a world-coordinate rectangle, in input order (newest first). */
    fun inside(points: List<WorldPoint>, minX: Double, minY: Double, maxX: Double, maxY: Double): List<WorldPoint> =
        points.filter { it.x in minX..maxX && it.y in minY..maxY }

    private class Acc(val cover: Long) {
        var n = 0
        var sx = 0.0
        var sy = 0.0
        var minX = Double.MAX_VALUE
        var minY = Double.MAX_VALUE
        var maxX = -Double.MAX_VALUE
        var maxY = -Double.MAX_VALUE
        fun add(p: WorldPoint) {
            n++; sx += p.x; sy += p.y
            minX = min(minX, p.x); minY = min(minY, p.y); maxX = max(maxX, p.x); maxY = max(maxY, p.y)
        }
        fun toCluster() = Cluster(cover, sx / n, sy / n, n, minX, minY, maxX, maxY)
    }
}
