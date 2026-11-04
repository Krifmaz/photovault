package app.photovault.android

import android.app.Application
import android.content.Context
import app.photovault.android.data.CoreProvider
import app.photovault.android.data.CoreRepository
import app.photovault.android.data.LanDiscovery
import app.photovault.android.data.NsdLanDiscovery
import app.photovault.android.data.TimelineStore
import app.photovault.android.data.VideoRepository
import app.photovault.android.data.VideoRepositoryProvider
import app.photovault.android.ui.viewer.video.ExoPlaybackEngine
import app.photovault.android.ui.viewer.video.PlaybackEngine
import app.photovault.android.ui.image.buildImageLoader
import app.photovault.android.ui.timeline.TimelineLabels
import coil3.ImageLoader
import coil3.PlatformContext
import app.photovault.android.data.NetworkWatcher
import coil3.SingletonImageLoader
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.launch
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

    /** Streamable videos (contract v1.2): core's loopback streams, or v1's `streamPlaylist` without them. */
    val videos: VideoRepository by lazy { VideoRepositoryProvider.create(core) }

    /** The viewer's player (one per open viewer, created on its first video, released when it closes). */
    fun newPlaybackEngine(): PlaybackEngine = ExoPlaybackEngine(context)

    /** App-lifetime work that must not block the UI or die with a screen (file cleanup, network re-checks). */
    val appScope: CoroutineScope = CoroutineScope(SupervisorJob() + Dispatchers.IO)

    /**
     * Decrypted originals for the viewer; app-private, trimmed by the viewer and cleared on sign-out.
     * Created (mkdirs) by the viewer on its IO dispatcher, never here: this is first read on the main thread.
     */
    val viewerCache: File = File(context.cacheDir, "viewer")

    /**
     * Drops every decrypted image of the current account from memory and disk: Coil's memory cache (keyed
     * by file id, which another server reuses) and the viewer's originals. Called on sign-out and whenever
     * a sign-in completes (another account or server may have been chosen). The timeline list itself is
     * cleared by sign-out and reloaded by the next timeline screen.
     */
    fun clearDecrypted() {
        SingletonImageLoader.get(context).memoryCache?.clear()
        appScope.launch { viewerCache.listFiles()?.forEach { it.delete() } }
    }

    /** Re-runs core's home/public address choice when the device's network changes. */
    fun watchNetwork() {
        NetworkWatcher(context, appScope) { core.refreshNetwork() }.start()
    }

    fun timelineLabels(): TimelineLabels =
        TimelineLabels(context.getString(R.string.timeline_today), context.getString(R.string.timeline_yesterday))
}

class PhotovaultApp : Application(), SingletonImageLoader.Factory {
    lateinit var graph: AppGraph
        private set

    override fun onCreate() {
        super.onCreate()
        graph = AppGraph(this).also {
            it.warmUp()
            it.watchNetwork()
        }
    }

    // The fetcher reads graph.core lazily on its IO dispatcher, so building the loader never opens core.
    override fun newImageLoader(context: PlatformContext): ImageLoader = buildImageLoader(this) { graph.core }
}

val Context.appGraph: AppGraph get() = (applicationContext as PhotovaultApp).graph
