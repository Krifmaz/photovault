package app.photovault.android.data.albums

import app.photovault.android.data.CoreRepository
import app.photovault.android.data.fake.FakeCore

/** Built without photovault-core bindings: the demo albums over the demo library. */
object AlbumsProvider {
    fun create(core: CoreRepository): AlbumsRepository = (core as? FakeCore)?.let { FakeAlbums(it) } ?: UnavailableAlbums
}
