package app.photovault.android.ui.albums

import app.photovault.android.data.albums.FakeAlbums
import app.photovault.android.data.fake.FakeCore
import kotlinx.coroutines.test.TestDispatcher

/** A signed-in demo core and its demo albums, on [dispatcher]. */
suspend fun signedInAlbums(dispatcher: TestDispatcher, now: Long = 1_793_664_000_000_000L): Pair<FakeCore, FakeAlbums> {
    val core = FakeCore(nowUs = now, itemCount = 640, latencyMs = 50, ioDispatcher = dispatcher)
    core.addServer("photos.example.com")
    core.login("me@example.com", "secret")
    return core to FakeAlbums(core)
}
