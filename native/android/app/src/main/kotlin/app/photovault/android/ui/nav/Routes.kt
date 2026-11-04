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

/** Camera backup settings and status. */
@Serializable
data object BackupRoute

/** Results handed back through the previous back stack entry's SavedStateHandle. */
object NavResults {
    const val QR_PAYLOAD = "qr_payload"
    const val VIEWER_INDEX = "viewer_index"
}
