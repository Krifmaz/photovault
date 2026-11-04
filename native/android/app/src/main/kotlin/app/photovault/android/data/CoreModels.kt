package app.photovault.android.data

/*
 * The app's view of photovault-core (contracts/photovault-core.md, contract v1). These mirror the UniFFI
 * records/enums one to one, with Kotlin types the UI can use directly (Long instead of ULong, ...).
 * The real adapter (src/realCore) converts the generated `app.photovault.core` types into these; FakeCore
 * produces them directly.
 */

/** `ServerInfo` record. [isPhotovault] false = a stock Ente-compatible server (answers /ping only). */
data class ServerInfo(
    val name: String,
    val version: String,
    val publicUrl: String,
    val homeUrl: String?,
    val signupsAllowed: Boolean,
    val isPhotovault: Boolean,
)

/** `StorageUsage` record. A null limit means unlimited. */
data class StorageUsage(val usedBytes: Long, val limitBytes: Long?, val freeBytes: Long?)

/** `Account` record. */
data class Account(val userId: Long, val email: String, val serverUrl: String)

/** `LoginStep` enum: what the sign-in needs next. */
sealed interface LoginStep {
    data class Done(val account: Account) : LoginStep
    data object EmailOtt : LoginStep
    data class TwoFactorTotp(val sessionId: String) : LoginStep
    data class Passkey(val sessionId: String, val url: String) : LoginStep
    data object NeedsRecoveryKey : LoginStep
}

/** `FileKind` enum. */
enum class FileKind { Image, Video, LivePhoto, Unknown }

/** `TimelineItem` record. Times are Unix microseconds (Ente's convention). */
data class TimelineItem(
    val id: Long,
    val collectionId: Long,
    val kind: FileKind,
    val takenAtUs: Long,
    val width: Int,
    val height: Int,
    val durationMs: Int?,
    val hasStream: Boolean,
    val title: String,
)

/** `MapPoint` record [contract v1.2]: a timeline file with a location (WGS 84 degrees). */
data class MapPoint(val fileId: Long, val latitude: Double, val longitude: Double, val takenAtUs: Long)

/** `Collection` record (named PhotoCollection so it does not shadow kotlin.collections.Collection). */
data class PhotoCollection(
    val id: Long,
    val name: String,
    val kind: String,
    val coverFileId: Long?,
    val count: Long,
    val updatedAtUs: Long,
)

/** `SyncProgress` record. */
data class SyncProgress(val collections: Int, val files: Long, val done: Boolean)

/** `UploadStatus` record. */
data class UploadStatus(val pending: Int, val uploading: Int, val failed: Int, val bytesDone: Long, val bytesTotal: Long)

/** `CoreError`: every failure core reports. */
sealed class CoreError(message: String) : Exception(message) {
    class Network(val detail: String) : CoreError(detail)
    class Unreachable(val address: String) : CoreError("Can't reach $address")
    class NotPhotovault(val address: String) : CoreError("$address is not a Photovault server")
    class Unauthorized : CoreError("Unauthorized")
    class IncorrectPassword : CoreError("Incorrect password")
    class IncorrectCode : CoreError("Incorrect code")
    class Server(val status: Int, val detail: String) : CoreError("Server error $status: $detail")
    class Crypto(val detail: String) : CoreError(detail)
    class Storage(val detail: String) : CoreError(detail)
    class NotLoggedIn : CoreError("Not signed in")
    class Cancelled : CoreError("Cancelled")
}
