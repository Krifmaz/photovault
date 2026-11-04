package app.photovault.android.data

import app.photovault.android.backup.BackupCore
import app.photovault.core.Core
import app.photovault.core.CoreException
import app.photovault.core.SyncListener
import app.photovault.core.discoverServers as coreDiscoverServers
import app.photovault.core.fetchServerInfo as coreFetchServerInfo
import app.photovault.core.parseConnectQr as coreParseConnectQr
import app.photovault.core.Account as CAccount
import app.photovault.core.Collection as CCollection
import app.photovault.core.FileKind as CFileKind
import app.photovault.core.LoginStep as CLoginStep
import app.photovault.core.ServerInfo as CServerInfo
import app.photovault.core.SyncProgress as CSyncProgress
import app.photovault.core.TimelineItem as CTimelineItem
import app.photovault.core.UploadStatus as CUploadStatus

/**
 * Adapter from the generated `app.photovault.core` API (contracts/photovault-core.md) to the app's types.
 * Every [CoreException] becomes the matching [CoreError].
 *
 * Core's async functions run on core's own tokio runtime, so the `suspend` functions here are main-safe.
 * The plain ones read core's local store (or the in-memory index) and are called from Dispatchers.IO.
 */
internal class RealCore(internal val core: Core) : CoreRepository, BackupCore {

    override fun discoverServers(timeoutMs: Int): List<String> = coreDiscoverServers(timeoutMs.coerceAtLeast(0).toUInt())

    override suspend fun fetchServerInfo(address: String): ServerInfo = guard(address) { coreFetchServerInfo(address).toApp() }

    override fun parseConnectQr(payload: String): String? = coreParseConnectQr(payload)

    override fun savedServers(): List<ServerInfo> = core.savedServers().map { it.toApp() }

    override suspend fun addServer(address: String): ServerInfo = guard(address) { core.addServer(address).toApp() }

    override fun removeServer(publicUrl: String) = guardSync { core.removeServer(publicUrl) }

    override fun selectServer(publicUrl: String) = guardSync { core.selectServer(publicUrl) }

    override fun activeBaseUrl(): String? = core.activeBaseUrl()

    override suspend fun refreshNetwork(): String? = core.refreshNetwork()

    override fun homeUrlAllowed(publicUrl: String): Boolean = core.homeUrlAllowed(publicUrl)

    override fun setHomeUrlAllowed(publicUrl: String, allowed: Boolean) = guardSync { core.setHomeUrlAllowed(publicUrl, allowed) }

    override suspend fun login(email: String, password: String): LoginStep = guard { core.login(email, password).toApp() }

    override suspend fun sendEmailOtt(email: String) = guard { core.sendEmailOtt(email) }

    override suspend fun verifyEmailOtt(email: String, code: String, password: String): LoginStep =
        guard { core.verifyEmailOtt(email, code, password).toApp() }

    override suspend fun verifyTotp(sessionId: String, code: String, password: String): LoginStep =
        guard { core.verifyTotp(sessionId, code, password).toApp() }

    override suspend fun verifyPasskey(sessionId: String, password: String): LoginStep =
        guard { core.verifyPasskey(sessionId, password).toApp() }

    override suspend fun recoverWithKey(recoveryKey: String, newPassword: String): Account =
        guard { core.recoverWithKey(recoveryKey, newPassword).toApp() }

    override fun currentAccount(): Account? = core.currentAccount()?.toApp()

    override suspend fun logout() = guard { core.logout() }

    override suspend fun storageUsage(): StorageUsage = guard {
        val u = core.storageUsage()
        StorageUsage(u.usedBytes.toLong(), u.limitBytes?.toLong(), u.freeBytes?.toLong())
    }

    override suspend fun sync(onProgress: ((SyncProgress) -> Unit)?): SyncProgress = guard {
        // Core calls the listener from its own worker thread; the app's callbacks only update StateFlows.
        val listener = onProgress?.let { cb ->
            object : SyncListener {
                override fun onProgress(p: CSyncProgress) = cb(p.toApp())
            }
        }
        core.sync(listener).toApp()
    }

    override fun timelineCount(): Long = core.timelineCount().toLong()

    override fun timelinePage(offset: Long, limit: Int): List<TimelineItem> =
        core.timelinePage(offset.coerceAtLeast(0).toULong(), limit.coerceAtLeast(0).toUInt()).map { it.toApp() }

    override fun collections(): List<PhotoCollection> = core.collections().map { it.toApp() }

    override fun collectionPage(collectionId: Long, offset: Long, limit: Int): List<TimelineItem> =
        core.collectionPage(collectionId, offset.coerceAtLeast(0).toULong(), limit.coerceAtLeast(0).toUInt()).map { it.toApp() }

    override fun mapPoints(): List<MapPoint> =
        core.mapPoints().map { MapPoint(it.fileId, it.latitude, it.longitude, it.takenAtUs) }

    override fun onThisDay(nowUs: Long, utcOffsetSecs: Int, limit: Int): List<TimelineItem> =
        core.onThisDay(nowUs, utcOffsetSecs, limit.coerceAtLeast(0).toUInt()).map { it.toApp() }

    override suspend fun thumbnail(fileId: Long): ByteArray = guard { core.thumbnail(fileId) }

    override suspend fun download(fileId: Long, destPath: String): String = guard { core.download(fileId, destPath) }

    override suspend fun streamPlaylist(fileId: Long): String? = guard { core.streamPlaylist(fileId) }

    override fun enqueueUpload(path: String, collectionId: Long?) = guardSync { core.enqueueUpload(path, collectionId) }

    override suspend fun runUploads(): UploadStatus = guard { core.runUploads().toApp() }

    override fun uploadStatus(): UploadStatus = core.uploadStatus().toApp()

    // camera backup (contract v1.2)
    override suspend fun backupAlbum(folderName: String): Long = guard { core.backupAlbum(folderName) }

    // ---- conversions ----

    private inline fun <T> guardSync(block: () -> T): T =
        try { block() } catch (e: CoreException) { throw e.toApp() }

    private suspend inline fun <T> guard(address: String? = null, crossinline block: suspend () -> T): T =
        try { block() } catch (e: CoreException) { throw e.toApp(address) }

    private fun CFileKind.toApp(): FileKind = when (this) {
        CFileKind.IMAGE -> FileKind.Image
        CFileKind.VIDEO -> FileKind.Video
        CFileKind.LIVE_PHOTO -> FileKind.LivePhoto
        CFileKind.UNKNOWN -> FileKind.Unknown
    }

    private fun CServerInfo.toApp() = ServerInfo(name, version, publicUrl, homeUrl, signupsAllowed, isPhotovault)

    private fun CAccount.toApp() = Account(userId, email, serverUrl)

    private fun CLoginStep.toApp(): LoginStep = when (this) {
        is CLoginStep.Done -> LoginStep.Done(account.toApp())
        is CLoginStep.TwoFactorTotp -> LoginStep.TwoFactorTotp(sessionId)
        is CLoginStep.Passkey -> LoginStep.Passkey(sessionId, url)
        CLoginStep.EmailOtt -> LoginStep.EmailOtt
        CLoginStep.NeedsRecoveryKey -> LoginStep.NeedsRecoveryKey
    }

    private fun CTimelineItem.toApp() = TimelineItem(
        id = id,
        collectionId = collectionId,
        kind = kind.toApp(),
        takenAtUs = takenAtUs,
        width = width.toInt(),
        height = height.toInt(),
        durationMs = durationMs?.toInt(),
        hasStream = hasStream,
        title = title,
    )

    private fun CCollection.toApp() =
        PhotoCollection(id, name, kind, coverFileId, count.toLong(), updatedAtUs)

    private fun CSyncProgress.toApp() = SyncProgress(collections.toInt(), files.toLong(), done)

    private fun CUploadStatus.toApp() =
        UploadStatus(pending.toInt(), uploading.toInt(), failed.toInt(), bytesDone.toLong(), bytesTotal.toLong())
}

/**
 * Generated `CoreException` -> app [CoreError]. In Kotlin the variants' `message` field is `detail`
 * (core/uniffi.toml renames it, since `message` clashes with Throwable). [address] is what the user typed,
 * shown in "Can't reach …" when core's own text is less readable.
 */
internal fun CoreException.toApp(address: String? = null): CoreError = when (this) {
    is CoreException.Network -> CoreError.Network(detail)
    is CoreException.Unreachable -> CoreError.Unreachable(address?.let(::displayAddress) ?: this.address)
    is CoreException.NotPhotovault -> CoreError.NotPhotovault(address?.let(::displayAddress) ?: this.address)
    is CoreException.Unauthorized -> CoreError.Unauthorized()
    is CoreException.IncorrectPassword -> CoreError.IncorrectPassword()
    is CoreException.IncorrectCode -> CoreError.IncorrectCode()
    is CoreException.Server -> CoreError.Server(status.toInt(), detail)
    is CoreException.Crypto -> CoreError.Crypto(detail)
    is CoreException.Storage -> CoreError.Storage(detail)
    is CoreException.NotLoggedIn -> CoreError.NotLoggedIn()
    is CoreException.Cancelled -> CoreError.Cancelled()
}
