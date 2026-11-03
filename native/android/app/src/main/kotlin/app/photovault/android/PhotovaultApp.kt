package app.photovault.android

import android.app.Application
import android.content.Context
import app.photovault.android.data.CoreProvider
import app.photovault.android.data.CoreRepository
import app.photovault.android.data.LanDiscovery
import app.photovault.android.data.NsdLanDiscovery
import app.photovault.android.data.TimelineStore
import app.photovault.android.ui.image.buildImageLoader
import app.photovault.android.ui.timeline.TimelineLabels
import coil3.ImageLoader
import coil3.PlatformContext
import coil3.SingletonImageLoader
import java.io.File

/** The app's few long-lived objects, created on first use. */
class AppGraph(private val context: Context) {
    /**
     * photovault-core. Opening it loads the native library and decrypts the local index, so it is warmed up
     * on a background thread at process start ([warmUp]); the UI reads it only from background dispatchers
     * (MainActivity resolves the start screen on Dispatchers.IO).
     */
    val core: CoreRepository by lazy { CoreProvider.create(context) }

    fun warmUp() {
        Thread({ core }, "photovault-core-init").apply { priority = Thread.NORM_PRIORITY }.start()
    }
    val discovery: LanDiscovery by lazy { NsdLanDiscovery(context) }
    val timeline: TimelineStore by lazy { TimelineStore(core) }

    /** Decrypted originals for the viewer; app-private, trimmed by the viewer and cleared on sign-out. */
    val viewerCache: File by lazy { File(context.cacheDir, "viewer").apply { mkdirs() } }

    fun timelineLabels(): TimelineLabels =
        TimelineLabels(context.getString(R.string.timeline_today), context.getString(R.string.timeline_yesterday))
}

class PhotovaultApp : Application(), SingletonImageLoader.Factory {
    lateinit var graph: AppGraph
        private set

    override fun onCreate() {
        super.onCreate()
        graph = AppGraph(this).also { it.warmUp() }
    }

    // The fetcher reads graph.core lazily on its IO dispatcher, so building the loader never opens core.
    override fun newImageLoader(context: PlatformContext): ImageLoader = buildImageLoader(this) { graph.core }
}

val Context.appGraph: AppGraph get() = (applicationContext as PhotovaultApp).graph
