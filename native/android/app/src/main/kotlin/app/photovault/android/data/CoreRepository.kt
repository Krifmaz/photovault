package app.photovault.android.data

/**
 * Everything the app asks of photovault-core: the free functions and the `Core` object of
 * contracts/photovault-core.md, same names in camelCase. Failures are thrown as [CoreError].
 *
 * Threading: `suspend` functions may do network/disk work and are main-safe (the implementation moves
 * off the main thread). The plain functions only read core's local store and are fast (< 5 ms for a
 * page of 200), but callers in the UI still invoke them from a background dispatcher.
 */
interface CoreRepository {
    // free functions
    fun discoverServers(timeoutMs: Int): List<String>
    suspend fun fetchServerInfo(address: String): ServerInfo
    fun parseConnectQr(payload: String): String?

    // servers (survive logout)
    fun savedServers(): List<ServerInfo>
    suspend fun addServer(address: String): ServerInfo
    fun removeServer(publicUrl: String)
    fun selectServer(publicUrl: String)
    fun activeBaseUrl(): String?
    suspend fun refreshNetwork(): String?
    /** Whether requests (sign-in, the session token) may go to this server's plain-http home URL. */
    fun homeUrlAllowed(publicUrl: String): Boolean
    fun setHomeUrlAllowed(publicUrl: String, allowed: Boolean)

    // auth
    suspend fun login(email: String, password: String): LoginStep
    suspend fun sendEmailOtt(email: String)
    suspend fun verifyEmailOtt(email: String, code: String, password: String): LoginStep
    suspend fun verifyTotp(sessionId: String, code: String, password: String): LoginStep
    /** After [LoginStep.Passkey]: returns the same Passkey step until the browser ceremony is done. */
    suspend fun verifyPasskey(sessionId: String, password: String): LoginStep
    suspend fun recoverWithKey(recoveryKey: String, newPassword: String): Account
    fun currentAccount(): Account?
    suspend fun logout()
    suspend fun storageUsage(): StorageUsage

    // library
    suspend fun sync(onProgress: ((SyncProgress) -> Unit)?): SyncProgress
    fun timelineCount(): Long
    fun timelinePage(offset: Long, limit: Int): List<TimelineItem>
    fun collections(): List<PhotoCollection>
    fun collectionPage(collectionId: Long, offset: Long, limit: Int): List<TimelineItem>
    suspend fun thumbnail(fileId: Long): ByteArray
    suspend fun download(fileId: Long, destPath: String): String
    suspend fun streamPlaylist(fileId: Long): String?

    // upload queue
    fun enqueueUpload(path: String, collectionId: Long?)
    suspend fun runUploads(): UploadStatus
    fun uploadStatus(): UploadStatus

    /** True for the in-memory demo implementation (shown as a hint in the UI). */
    val isDemo: Boolean get() = false
}
