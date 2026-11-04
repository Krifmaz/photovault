package app.photovault.android.ui.image

import android.content.Context
import app.photovault.android.data.CoreRepository
import app.photovault.android.data.ml.MlRepository
import coil3.ImageLoader
import coil3.decode.DataSource
import coil3.decode.ImageSource
import coil3.disk.DiskCache
import coil3.fetch.FetchResult
import coil3.fetch.Fetcher
import coil3.fetch.SourceFetchResult
import coil3.key.Keyer
import coil3.memory.MemoryCache
import coil3.request.Options
import coil3.request.crossfade
import kotlinx.coroutines.CoroutineDispatcher
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import okio.Buffer
import okio.Path.Companion.toOkioPath

/** Coil model for a file's thumbnail, decrypted by core. */
data class Thumb(val fileId: Long)

/**
 * Loads [Thumb]s through core's `thumbnail(id)` on [io]. Core keeps its own thumbnail cache encrypted at
 * rest, so decrypted bytes go only to Coil's memory cache (as bitmaps decoded at the requested size),
 * never to Coil's disk cache.
 */
class ThumbnailFetcher(
    private val data: Thumb,
    private val options: Options,
    private val core: () -> CoreRepository,
    private val io: CoroutineDispatcher,
) : Fetcher {
    override suspend fun fetch(): FetchResult {
        val bytes = withContext(io) { core().thumbnail(data.fileId) }
        val buffer = Buffer().write(bytes)
        return SourceFetchResult(
            source = ImageSource(buffer, options.fileSystem),
            mimeType = null,
            dataSource = DataSource.DISK,
        )
    }

    class Factory(private val core: () -> CoreRepository, private val io: CoroutineDispatcher = Dispatchers.IO) : Fetcher.Factory<Thumb> {
        override fun create(data: Thumb, options: Options, imageLoader: ImageLoader): Fetcher =
            ThumbnailFetcher(data, options, core, io)
    }
}

/** Memory-cache key for thumbnails (Coil only memory-caches models that have a key). */
class ThumbKeyer : Keyer<Thumb> {
    override fun key(data: Thumb, options: Options): String = "thumb:${data.fileId}"
}

/** The app's single ImageLoader: thumbnails and face crops from core, full-size files from the viewer cache. */
fun buildImageLoader(context: Context, core: () -> CoreRepository, ml: (() -> MlRepository)? = null): ImageLoader =
    ImageLoader.Builder(context)
        .components {
            add(ThumbnailFetcher.Factory(core))
            add(ThumbKeyer())
            if (ml != null) {
                add(FaceThumbnailFetcher.Factory(ml))
                add(FaceThumbKeyer())
            }
        }
        .memoryCache {
            MemoryCache.Builder()
                .maxSizePercent(context, 0.25)
                .build()
        }
        .diskCache {
            // Only for models Coil itself fetches from the network (none today); thumbnails bypass it on purpose.
            DiskCache.Builder()
                .directory(context.cacheDir.resolve("coil").toOkioPath())
                .maxSizeBytes(64L * 1024 * 1024)
                .build()
        }
        .crossfade(120)
        .build()
