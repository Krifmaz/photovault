package app.photovault.android.ui.image

import app.photovault.android.data.ml.MlRepository
import coil3.ImageLoader
import coil3.decode.DataSource
import coil3.decode.ImageSource
import coil3.fetch.FetchResult
import coil3.fetch.Fetcher
import coil3.fetch.SourceFetchResult
import coil3.key.Keyer
import coil3.request.Options
import kotlinx.coroutines.CoroutineDispatcher
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import okio.Buffer

/** Coil model for a face crop (an Ente face id), cut and decrypted by core's `face_thumbnail`. */
data class FaceThumb(val faceId: String)

/**
 * Loads [FaceThumb]s through core's `faceThumbnail(faceId)` on [io], like [ThumbnailFetcher]: core keeps its
 * crops encrypted at rest, so the decrypted JPEG only reaches Coil's memory cache (decoded at avatar size),
 * never its disk cache.
 */
class FaceThumbnailFetcher(
    private val data: FaceThumb,
    private val options: Options,
    private val ml: () -> MlRepository,
    private val io: CoroutineDispatcher,
) : Fetcher {
    override suspend fun fetch(): FetchResult {
        val bytes = withContext(io) { ml().faceThumbnail(data.faceId) }
        return SourceFetchResult(
            source = ImageSource(Buffer().write(bytes), options.fileSystem),
            mimeType = null,
            dataSource = DataSource.DISK,
        )
    }

    class Factory(private val ml: () -> MlRepository, private val io: CoroutineDispatcher = Dispatchers.IO) : Fetcher.Factory<FaceThumb> {
        override fun create(data: FaceThumb, options: Options, imageLoader: ImageLoader): Fetcher =
            FaceThumbnailFetcher(data, options, ml, io)
    }
}

/** Memory-cache key for face crops. */
class FaceThumbKeyer : Keyer<FaceThumb> {
    override fun key(data: FaceThumb, options: Options): String = "face:${data.faceId}"
}
