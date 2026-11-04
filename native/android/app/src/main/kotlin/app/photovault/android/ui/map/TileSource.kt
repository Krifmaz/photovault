package app.photovault.android.ui.map

import android.content.Context
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.util.LruCache
import app.photovault.android.BuildConfig
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.launch
import kotlinx.coroutines.sync.Semaphore
import kotlinx.coroutines.sync.withPermit
import java.io.File
import java.io.IOException
import java.net.HttpURLConnection
import java.net.URL
import java.util.concurrent.ConcurrentHashMap

/** One map tile: x, y at zoom z (XYZ / "slippy map" numbering). */
data class TileKey(val z: Int, val x: Int, val y: Int) {
    val packed: Long get() = (z.toLong() shl 58) or (x.toLong() shl 29) or y.toLong()
    fun parent(): TileKey? = if (z == 0) null else TileKey(z - 1, x shr 1, y shr 1)
}

/**
 * Raster map tiles, as Ente's apps draw their map: OpenStreetMap's standard tiles
 * (`https://tile.openstreetmap.org/{z}/{x}/{y}.png`), overridable at build time with
 * `-Pphotovault.mapTiles=<url template>` (for a self-hosted tile server).
 *
 * Tiles are public map pictures, not the user's data, so they are cached on disk as downloaded (capped at
 * [DISK_MAX] and [MAX_AGE_MS], as OSM's tile usage policy asks), plus a memory cache of decoded bitmaps.
 * Requests carry an identifying User-Agent and at most [PARALLEL] run at once. Nothing about the user's
 * photos is sent: only the tile coordinates the map shows.
 */
class TileSource(context: Context, private val template: String = BuildConfig.MAP_TILE_URL) {
    private val dir = File(context.cacheDir, "map-tiles")
    private val memory = object : LruCache<Long, Bitmap>(MEMORY_MAX) {
        override fun sizeOf(key: Long, value: Bitmap): Int = value.allocationByteCount
    }
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
    private val inflight = ConcurrentHashMap<Long, Job>()
    private val gate = Semaphore(PARALLEL)
    private val userAgent = "PhotovaultNative/${BuildConfig.VERSION_NAME} (Android; self-hosted photo app)"

    @Volatile private var trimmed = false

    /** The decoded tile if it is in memory; null otherwise (then [request] it). */
    fun cached(key: TileKey): Bitmap? = memory.get(key.packed)

    /** Starts loading [key] (disk, then network) unless it is loaded or loading; [onReady] runs on a worker thread. */
    fun request(key: TileKey, onReady: () -> Unit) {
        if (memory.get(key.packed) != null || inflight.containsKey(key.packed)) return
        val job = scope.launch(start = kotlinx.coroutines.CoroutineStart.LAZY) {
            try {
                val bmp = gate.withPermit { load(key) }
                if (bmp != null) {
                    memory.put(key.packed, bmp)
                    onReady()
                }
            } finally {
                inflight.remove(key.packed)
            }
        }
        if (inflight.putIfAbsent(key.packed, job) == null) job.start() else job.cancel()
    }

    /** Drops requests for tiles no longer on screen (fast panning would otherwise queue hundreds). */
    fun retain(wanted: Set<Long>) {
        for ((k, job) in inflight) if (k !in wanted) job.cancel()
    }

    fun close() {
        scope.cancel()
        memory.evictAll()
    }

    private fun file(key: TileKey) = File(dir, "${key.z}/${key.x}/${key.y}.png")

    private fun load(key: TileKey): Bitmap? {
        if (!trimmed) { trimmed = true; trimDisk() }
        val f = file(key)
        if (f.isFile && System.currentTimeMillis() - f.lastModified() < MAX_AGE_MS) {
            BitmapFactory.decodeFile(f.path)?.let { return it }
        }
        val bytes = download(key) ?: return f.takeIf { it.isFile }?.let { BitmapFactory.decodeFile(it.path) }
        val bmp = BitmapFactory.decodeByteArray(bytes, 0, bytes.size) ?: return null
        try {
            f.parentFile?.mkdirs()
            val tmp = File(f.parentFile, f.name + ".part")
            tmp.writeBytes(bytes)
            if (!tmp.renameTo(f)) tmp.delete()
        } catch (_: IOException) {
            // a full cache only costs a re-download
        }
        return bmp
    }

    private fun download(key: TileKey): ByteArray? {
        val url = template
            .replace("{z}", key.z.toString())
            .replace("{x}", key.x.toString())
            .replace("{y}", key.y.toString())
        return try {
            val c = URL(url).openConnection() as HttpURLConnection
            c.connectTimeout = 10_000
            c.readTimeout = 15_000
            c.setRequestProperty("User-Agent", userAgent)
            try {
                if (c.responseCode != 200) null else c.inputStream.use { it.readBytes() }
            } finally {
                c.disconnect()
            }
        } catch (_: IOException) {
            null
        }
    }

    /** Oldest tiles go first once the cache is over [DISK_MAX]. */
    private fun trimDisk() {
        val files = dir.walkTopDown().filter { it.isFile }.toList()
        var total = files.sumOf { it.length() }
        if (total <= DISK_MAX) return
        for (f in files.sortedBy { it.lastModified() }) {
            total -= f.length()
            f.delete()
            if (total <= DISK_MAX * 3 / 4) break
        }
    }

    companion object {
        const val PARALLEL = 4
        const val MEMORY_MAX = 48 * 1024 * 1024
        const val DISK_MAX = 128L * 1024 * 1024
        const val MAX_AGE_MS = 7L * 24 * 3600 * 1000
    }
}
