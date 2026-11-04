package app.photovault.android

import android.app.Application
import android.content.Context
import app.photovault.android.data.CoreProvider
import app.photovault.android.data.CoreRepository
import app.photovault.android.data.LanDiscovery
import app.photovault.android.data.NsdLanDiscovery
import app.photovault.android.data.TimelineStore
import app.photovault.android.data.ml.MlRepository
import app.photovault.android.ui.image.buildImageLoader
import app.photovault.android.ui.memories.MemoryFeed
import app.photovault.android.ui.search.ResultLists
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

    /**
     * Search, people and memories (contract v1.2) over the same core: real with RealCore, the demo data with
     * FakeCore, an unavailable stand-in with UnavailableCore. Reading it opens [core], so like [core] it is
     * first read from background dispatchers (the screens' view models and Coil's face fetcher).
     */
    val ml: MlRepository by lazy { CoreProvider.createMl(core) }

    /** Today's memories, shared by the timeline's strip and the story viewer (reads [ml] only on refresh). */
    val memoryFeed: MemoryFeed by lazy { MemoryFeed({ ml }) }

    fun warmUp() {
        Thread({ core }, "photovault-core-init").apply { priority = Thread.NORM_PRIORITY }.start()
    }
    val discovery: LanDiscovery by lazy { NsdLanDiscovery(context) }
    val timeline: TimelineStore by lazy { TimelineStore(core) }

    /** App-lifetime work that must not block the UI or die with a screen (file cleanup, network re-checks). */
    val appScope: CoroutineScope = CoroutineScope(SupervisorJob() + Dispatchers.IO)

    /**
     * Decrypted originals for the viewer; app-private, trimmed by the viewer and cleared on sign-out.
     * Created (mkdirs) by the viewer on its IO dispatcher, never here: this is first read on the main thread.
     */
    val viewerCache: File = File(context.cacheDir, "viewer")

    /** Decrypted originals of the open memory story (at most a few; created by its view model on IO). */
    val memoryCache: File = File(context.cacheDir, "memories")

    /**
     * Drops every decrypted image of the current account from memory and disk: Coil's memory cache (keyed
     * by file id, which another server reuses) and the viewer's originals. Called on sign-out and whenever
     * a sign-in completes (another account or server may have been chosen). The timeline list itself is
     * cleared by sign-out and reloaded by the next timeline screen.
     */
    fun clearDecrypted() {
        SingletonImageLoader.get(context).memoryCache?.clear()
        ResultLists.clear()
        memoryFeed.clear()
        appScope.launch {
            viewerCache.listFiles()?.forEach { it.delete() }
            memoryCache.listFiles()?.forEach { it.delete() }
        }
    }

    /**
     * Deletes decrypted originals a previous process left behind (killed while the viewer or a memory story
     * was open, so their view models never cleaned up). Runs at process start, before any screen exists.
     */
    fun wipeLeftovers() {
        appScope.launch {
            viewerCache.listFiles()?.forEach { it.delete() }
            memoryCache.listFiles()?.forEach { it.delete() }
        }
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
            it.wipeLeftovers()
            it.watchNetwork()
        }
    }

    // The fetchers read graph.core / graph.ml lazily on their IO dispatcher, so building the loader never opens core.
    override fun newImageLoader(context: PlatformContext): ImageLoader = buildImageLoader(this, core = { graph.core }, ml = { graph.ml })
}

val Context.appGraph: AppGraph get() = (applicationContext as PhotovaultApp).graph
