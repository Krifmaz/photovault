package app.photovault.android.data

/** Built without photovault-core bindings: streams via the demo library's `streamPlaylist` (none). */
object VideoRepositoryProvider {
    fun create(core: CoreRepository): VideoRepository = PlaylistVideoRepository(core)
}
