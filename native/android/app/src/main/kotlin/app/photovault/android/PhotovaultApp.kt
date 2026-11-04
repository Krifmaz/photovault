package app.photovault.android

import android.app.Application
import android.content.Context
import app.photovault.android.backup.BackupGraph
import app.photovault.android.data.CoreProvider
import app.photovault.android.data.CoreRepository
import app.photovault.android.data.LanDiscovery
import app.photovault.android.data.NsdLanDiscovery
import app.photovault.android.data.SignupProvider
import app.photovault.android.data.SignupRepository
import app.photovault.android.data.TimelineStore
import app.photovault.android.data.albums.AlbumsProvider
import app.photovault.android.data.albums.AlbumsRepository
import app.photovault.android.ui.image.buildImageLoader
import app.photovault.android.ui.timeline.TimelineLabels
import app.photovault.android.widget.WidgetRefresh
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
    /** Create account (contract v1.2 signup), on the same core. */
    val signup: SignupRepository by lazy { SignupProvider.create(context, core) }

    /** Camera backup (WorkManager): settings, media scan, ledger, scheduling. */
    val backup: BackupGraph by lazy { BackupGraph(context) { core } }

    /** Albums, sharing, hidden albums and trash (photovault-core's albums API, or the demo library). */
    val albums: AlbumsRepository by lazy { AlbumsProvider.create(core) }

    private var albumStore: Pair<Long, TimelineStore>? = null

    /**
     * The loaded photos of one album, shared by its grid and the viewer opened from it (like [timeline]).
     * Only the most recently opened album is kept.
     */
    fun albumStore(collectionId: Long): TimelineStore = synchronized(this) {
        albumStore?.takeIf { it.first == collectionId }?.second
            ?: TimelineStore(
                core,
                countOf = { albums.albumCount(collectionId) },
                pageOf = { offset, limit -> albums.albumPage(collectionId, offset, limit) },
            ).also { albumStore = collectionId to it }
    }

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
        // widgets of the old account must not keep showing its photos
        refreshWidgets()
    }

    /** Re-renders the home-screen widgets (after a sync, sign-in or sign-out). */
    fun refreshWidgets() = WidgetRefresh.all(context)

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
            it.appScope.launch { it.backup.start() }
        }
    }

    // The fetcher reads graph.core lazily on its IO dispatcher, so building the loader never opens core.
    override fun newImageLoader(context: PlatformContext): ImageLoader = buildImageLoader(this) { graph.core }
}

val Context.appGraph: AppGraph get() = (applicationContext as PhotovaultApp).graph
