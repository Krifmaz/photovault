package app.photovault.android.data

/**
 * Stands in for photovault-core when its native library could not be loaded in a release build: nothing
 * is saved or signed in, and every network or library call fails with a [CoreError.Storage] that names the
 * problem, so the connect screen shows it instead of a crash or fake content.
 */
class UnavailableCore(private val reason: String) : CoreRepository {
    private fun fail(): Nothing = throw CoreError.Storage("This app's encryption library could not be loaded ($reason).")

    override fun discoverServers(timeoutMs: Int): List<String> = emptyList()
    override suspend fun fetchServerInfo(address: String): ServerInfo = fail()
    override fun parseConnectQr(payload: String): String? = ConnectQr.address(payload)

    override fun savedServers(): List<ServerInfo> = emptyList()
    override suspend fun addServer(address: String): ServerInfo = fail()
    override fun removeServer(publicUrl: String) = Unit
    override fun selectServer(publicUrl: String): Unit = fail()
    override fun activeBaseUrl(): String? = null
    override suspend fun refreshNetwork(): String? = null
    override fun homeUrlAllowed(publicUrl: String): Boolean = false
    override fun setHomeUrlAllowed(publicUrl: String, allowed: Boolean): Unit = fail()

    override suspend fun login(email: String, password: String): LoginStep = fail()
    override suspend fun sendEmailOtt(email: String): Unit = fail()
    override suspend fun verifyEmailOtt(email: String, code: String, password: String): LoginStep = fail()
    override suspend fun verifyTotp(sessionId: String, code: String, password: String): LoginStep = fail()
    override suspend fun verifyPasskey(sessionId: String, password: String): LoginStep = fail()
    override suspend fun recoverWithKey(recoveryKey: String, newPassword: String): Account = fail()
    override fun currentAccount(): Account? = null
    override suspend fun logout() = Unit
    override suspend fun storageUsage(): StorageUsage = fail()

    override suspend fun sync(onProgress: ((SyncProgress) -> Unit)?): SyncProgress = fail()
    override fun timelineCount(): Long = 0
    override fun timelinePage(offset: Long, limit: Int): List<TimelineItem> = emptyList()
    override fun collections(): List<PhotoCollection> = emptyList()
    override fun collectionPage(collectionId: Long, offset: Long, limit: Int): List<TimelineItem> = emptyList()
    override fun mapPoints(): List<MapPoint> = emptyList()
    override fun onThisDay(nowUs: Long, utcOffsetSecs: Int, limit: Int): List<TimelineItem> = emptyList()
    override suspend fun thumbnail(fileId: Long): ByteArray = fail()
    override suspend fun download(fileId: Long, destPath: String): String = fail()
    override suspend fun streamPlaylist(fileId: Long): String? = null

    override fun enqueueUpload(path: String, collectionId: Long?): Unit = fail()
    override suspend fun runUploads(): UploadStatus = fail()
    override fun uploadStatus(): UploadStatus = UploadStatus(0, 0, 0, 0, 0)
}
