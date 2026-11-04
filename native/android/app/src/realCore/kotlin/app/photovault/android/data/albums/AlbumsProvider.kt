package app.photovault.android.data.albums

import app.photovault.android.data.CoreRepository
import app.photovault.android.data.RealCore
import app.photovault.android.data.fake.FakeCore

/** Albums over the same core the app uses: photovault-core when it loaded, else the demo library. */
object AlbumsProvider {
    fun create(core: CoreRepository): AlbumsRepository = when (core) {
        is RealCore -> RealAlbums(core.core)
        is FakeCore -> FakeAlbums(core)
        else -> UnavailableAlbums
    }
}
