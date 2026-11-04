package app.photovault.android.data

/** Built with photovault-core: core's streams when [CoreProvider] chose the real core. */
object VideoRepositoryProvider {
    fun create(core: CoreRepository): VideoRepository =
        if (core is RealCore) RealVideoRepository(core.core) else PlaylistVideoRepository(core)
}
