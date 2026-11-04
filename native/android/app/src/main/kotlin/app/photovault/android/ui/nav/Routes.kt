package app.photovault.android.ui.nav

import kotlinx.serialization.Serializable

@Serializable
data object ConnectRoute

@Serializable
data object QrScanRoute

/** Nested graph of the sign-in screens; [app.photovault.android.ui.signin.SignInViewModel] is scoped to it. */
@Serializable
data object AuthGraph

@Serializable
data object SignInRoute

@Serializable
data object EmailCodeRoute

@Serializable
data class TotpRoute(val sessionId: String)

@Serializable
data class PasskeyRoute(val sessionId: String, val url: String)

@Serializable
data object RecoveryRoute

@Serializable
data object TimelineRoute

/** [index] is the position in the timeline, [fileId] the photo (its shared-element key). */
@Serializable
data class ViewerRoute(val index: Int, val fileId: Long)

/** A memory as a full-screen story, opened at photo [index]. */
@Serializable
data class MemoryRoute(val memoryId: String, val index: Int = 0)

/** The photo viewer over one memory's photos ([index] in the memory, [fileId] the photo). */
@Serializable
data class MemoryViewerRoute(val memoryId: String, val index: Int, val fileId: Long)

/** Machine learning settings (faces and magic search). */
@Serializable
data object MlSettingsRoute

/** Photos on a map. */
@Serializable
data object MapRoute

/** Camera backup settings and status. */
@Serializable
data object BackupRoute

/** The Albums tab (bottom bar). */
@Serializable
data object AlbumsRoute

@Serializable
data class AlbumRoute(val collectionId: Long)

/** The viewer over one album's photos; [index] is the position in that album. */
@Serializable
data class AlbumViewerRoute(val collectionId: Long, val index: Int, val fileId: Long)

@Serializable
data class ShareAlbumRoute(val collectionId: Long)

@Serializable
data object HiddenRoute

@Serializable
data object TrashRoute

/** Results handed back through the previous back stack entry's SavedStateHandle. */
object NavResults {
    const val QR_PAYLOAD = "qr_payload"
    const val VIEWER_INDEX = "viewer_index"
    const val MEMORY_INDEX = "memory_index"
}
