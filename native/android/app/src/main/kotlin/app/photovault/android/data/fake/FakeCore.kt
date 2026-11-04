package app.photovault.android.data.fake

import app.photovault.android.data.Account
import app.photovault.android.data.ConnectQr
import app.photovault.android.data.CoreError
import app.photovault.android.data.CoreRepository
import app.photovault.android.data.FileKind
import app.photovault.android.data.LoginStep
import app.photovault.android.data.MapPoint
import app.photovault.android.data.PhotoCollection
import app.photovault.android.data.ServerInfo
import app.photovault.android.data.StorageUsage
import app.photovault.android.data.SyncProgress
import app.photovault.android.data.TimelineItem
import app.photovault.android.data.UploadStatus
import app.photovault.android.data.displayAddress
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.withContext
import java.io.File
import java.net.URI
import kotlin.math.max
import kotlin.math.roundToInt

/**
 * In-memory stand-in for photovault-core, used when the UniFFI bindings are not built (and by the unit tests).
 * Nothing leaves the process: no network, no crypto, no disk except [download] writing a generated picture.
 *
 * Demo rules (so every screen can be reached):
 * - any address connects, except hosts containing "unreachable" or ending in ".invalid" ([CoreError.Unreachable]),
 *   "notphotovault" ([CoreError.NotPhotovault]) and "compat" (a stock-compatible server, isPhotovault = false);
 * - any non-empty password signs in, "wrong" gives [CoreError.IncorrectPassword]; an email containing
 *   "+otp", "+totp", "+passkey" or "+recovery" asks for that step; the code "123456" is accepted.
 */
class FakeCore(
    private val nowUs: Long = System.currentTimeMillis() * 1000L,
    itemCount: Int = 640,
    private val latencyMs: Long = 350L,
    private val ioDispatcher: kotlinx.coroutines.CoroutineDispatcher = Dispatchers.IO,
) : CoreRepository {

    override val isDemo: Boolean get() = true

    private val lock = Any()
    private val servers = LinkedHashMap<String, ServerInfo>()
    private var active: String? = null
    private var account: Account? = null
    private var pendingUploads = 0
    private val homeAllowed = HashSet<String>()
    private var passkeyChecks = 0

    private val collections: List<PhotoCollection>
    private val items: List<TimelineItem>

    init {
        val rnd = PlaceholderPng.Lcg(42)
        val albumNames = listOf("Camera", "Holidays", "Family", "Screenshots", "Hikes")
        val list = ArrayList<TimelineItem>(itemCount)
        var t = nowUs - 2 * HOUR_US
        for (i in 0 until itemCount) {
            // bursts: several shots minutes apart, then a gap of hours to days
            t -= if (rnd.nextFloat() < 0.7f) (1 + rnd.nextFloat() * 20).roundToInt() * MINUTE_US
            else (3 + rnd.nextFloat() * 60).roundToInt() * HOUR_US
            val roll = rnd.nextFloat()
            val kind = when {
                roll < 0.07f -> FileKind.Video
                roll < 0.10f -> FileKind.LivePhoto
                else -> FileKind.Image
            }
            val (w, h) = when {
                kind == FileKind.Video -> if (rnd.nextFloat() < 0.5f) 1920 to 1080 else 1080 to 1920
                rnd.nextFloat() < 0.6f -> 4032 to 3024
                rnd.nextFloat() < 0.8f -> 3024 to 4032
                else -> 3000 to 3000
            }
            val id = 10_000L + i
            list += TimelineItem(
                id = id,
                collectionId = 1L + (i % albumNames.size),
                kind = kind,
                takenAtUs = t,
                width = w,
                height = h,
                durationMs = if (kind == FileKind.Video) (3_000 + rnd.nextFloat() * 90_000).roundToInt() else null,
                hasStream = kind == FileKind.Video && rnd.nextFloat() < 0.5f,
                title = "IMG_%04d.%s".format(i + 1, if (kind == FileKind.Video) "mp4" else "jpg"),
            )
        }
        items = list
        collections = albumNames.mapIndexed { index, name ->
            val inAlbum = list.filter { it.collectionId == index + 1L }
            PhotoCollection(
                id = index + 1L,
                name = name,
                kind = "album",
                coverFileId = inAlbum.firstOrNull()?.id,
                count = inAlbum.size.toLong(),
                updatedAtUs = inAlbum.firstOrNull()?.takenAtUs ?: nowUs,
            )
        }
    }

    // ---- free functions ----

    override fun discoverServers(timeoutMs: Int): List<String> = emptyList()

    override suspend fun fetchServerInfo(address: String): ServerInfo {
        delay(latencyMs)
        val url = normalize(address)
        val host = runCatching { URI(url).host }.getOrNull()?.lowercase()
            ?: throw CoreError.Network("\"${address.trim()}\" is not a server address")
        if ("unreachable" in host || host.endsWith(".invalid")) throw CoreError.Unreachable(displayAddress(address))
        if ("notphotovault" in host) throw CoreError.NotPhotovault(displayAddress(address))
        if ("compat" in host) return ServerInfo("", "", url, null, signupsAllowed = true, isPhotovault = false)
        val isLan = host.startsWith("10.") || host.startsWith("192.168.") || host.endsWith(".local")
        return ServerInfo(
            name = "Demo vault",
            version = "0.2.0",
            publicUrl = url,
            homeUrl = if (isLan) url else null,
            signupsAllowed = false,
            isPhotovault = true,
        )
    }

    override fun parseConnectQr(payload: String): String? = ConnectQr.address(payload)

    // ---- servers ----

    override fun savedServers(): List<ServerInfo> = synchronized(lock) { servers.values.toList() }

    override suspend fun addServer(address: String): ServerInfo {
        val info = fetchServerInfo(address)
        synchronized(lock) {
            servers[info.publicUrl] = info
            active = info.publicUrl
        }
        return info
    }

    override fun removeServer(publicUrl: String) {
        synchronized(lock) {
            servers.remove(publicUrl)
            if (active == publicUrl) {
                active = null
                account = null
            }
        }
    }

    override fun selectServer(publicUrl: String) {
        synchronized(lock) {
            if (publicUrl !in servers) throw CoreError.Storage("Unknown server $publicUrl")
            if (active != publicUrl) account = null
            active = publicUrl
        }
    }

    override fun activeBaseUrl(): String? = synchronized(lock) {
        active?.let { a -> servers[a]?.homeUrl?.takeIf { it.startsWith("https://") || a in homeAllowed } ?: a }
    }

    override suspend fun refreshNetwork(): String? = activeBaseUrl()

    override fun homeUrlAllowed(publicUrl: String): Boolean = synchronized(lock) { publicUrl in homeAllowed }

    override fun setHomeUrlAllowed(publicUrl: String, allowed: Boolean) {
        synchronized(lock) { if (allowed) homeAllowed += publicUrl else homeAllowed -= publicUrl }
    }

    // ---- auth ----

    override suspend fun login(email: String, password: String): LoginStep {
        delay(latencyMs)
        val server = synchronized(lock) { active } ?: throw CoreError.Network("Choose a server first")
        if (password.isEmpty() || password == "wrong") throw CoreError.IncorrectPassword()
        val e = email.trim().lowercase()
        return when {
            "+totp" in e -> LoginStep.TwoFactorTotp("fake-totp-session")
            "+passkey" in e -> LoginStep.Passkey("fake-passkey-session", "$server/passkeys?session=fake")
            "+otp" in e -> LoginStep.EmailOtt
            "+recovery" in e -> LoginStep.NeedsRecoveryKey
            else -> LoginStep.Done(signIn(e, server))
        }
    }

    override suspend fun sendEmailOtt(email: String) {
        delay(latencyMs)
    }

    override suspend fun verifyEmailOtt(email: String, code: String, password: String): LoginStep {
        delay(latencyMs)
        if (code.trim() != DEMO_CODE) throw CoreError.IncorrectCode()
        val server = synchronized(lock) { active } ?: throw CoreError.NotLoggedIn()
        return when {
            "+totp" in email.lowercase() -> LoginStep.TwoFactorTotp(if (password.isEmpty()) "fake-totp-recovery" else "fake-totp-session")
            // like core: a code verified without a password ("Use recovery key") asks for the recovery key
            password.isEmpty() -> LoginStep.NeedsRecoveryKey
            else -> LoginStep.Done(signIn(email.trim().lowercase(), server))
        }
    }

    override suspend fun verifyTotp(sessionId: String, code: String, password: String): LoginStep {
        delay(latencyMs)
        if (code.trim() != DEMO_CODE) throw CoreError.IncorrectCode()
        val server = synchronized(lock) { active } ?: throw CoreError.NotLoggedIn()
        if (password.isEmpty()) return LoginStep.NeedsRecoveryKey
        return LoginStep.Done(signIn("demo@photovault.app", server))
    }

    /** The demo passkey ceremony "finishes" on the second check. */
    override suspend fun verifyPasskey(sessionId: String, password: String): LoginStep {
        delay(latencyMs)
        val server = synchronized(lock) { active } ?: throw CoreError.NotLoggedIn()
        val checks = synchronized(lock) { ++passkeyChecks }
        if (checks < 2) return LoginStep.Passkey(sessionId, "$server/passkeys?session=fake")
        synchronized(lock) { passkeyChecks = 0 }
        if (password.isEmpty()) return LoginStep.NeedsRecoveryKey
        return LoginStep.Done(signIn("demo@photovault.app", server))
    }

    override suspend fun recoverWithKey(recoveryKey: String, newPassword: String): Account {
        delay(latencyMs)
        val words = recoveryKey.trim().split(Regex("\\s+")).filter { it.isNotEmpty() }
        if (words.size != 24 && recoveryKey.trim().length != 64) throw CoreError.Crypto("That recovery key is not valid")
        val server = synchronized(lock) { active } ?: throw CoreError.NotLoggedIn()
        return signIn("demo@photovault.app", server)
    }

    private fun signIn(email: String, server: String): Account =
        Account(userId = 1, email = email.replace(Regex("\\+[a-z]+"), ""), serverUrl = server).also {
            synchronized(lock) { account = it }
        }

    override fun currentAccount(): Account? = synchronized(lock) { account }

    override suspend fun logout() {
        synchronized(lock) { account = null }
    }

    override suspend fun storageUsage(): StorageUsage {
        requireAccount()
        val used = items.sumOf { if (it.kind == FileKind.Video) 40_000_000L else 3_500_000L }
        val limit = 200L * 1024 * 1024 * 1024
        return StorageUsage(used, limit, limit - used)
    }

    // ---- library ----

    override suspend fun sync(onProgress: ((SyncProgress) -> Unit)?): SyncProgress {
        requireAccount()
        val steps = 4
        for (s in 1..steps) {
            delay(latencyMs / 2)
            onProgress?.invoke(SyncProgress(collections.size, items.size.toLong() * s / steps, s == steps))
        }
        return SyncProgress(collections.size, items.size.toLong(), true)
    }

    override fun timelineCount(): Long = if (currentAccount() == null) 0 else items.size.toLong()

    override fun timelinePage(offset: Long, limit: Int): List<TimelineItem> {
        if (currentAccount() == null || offset >= items.size || limit <= 0) return emptyList()
        val from = offset.toInt()
        return items.subList(from, minOf(items.size, from + limit)).toList()
    }

    override fun collections(): List<PhotoCollection> = if (currentAccount() == null) emptyList() else collections

    override fun collectionPage(collectionId: Long, offset: Long, limit: Int): List<TimelineItem> {
        if (currentAccount() == null) return emptyList()
        return items.asSequence().filter { it.collectionId == collectionId }.drop(offset.toInt()).take(limit).toList()
    }

    /** Two in three demo photos have a place: around a few cities, spread a little. */
    override fun mapPoints(): List<MapPoint> {
        if (currentAccount() == null) return emptyList()
        return items.filter { it.id % 3 != 0L }.map { item ->
            val (lat, lon) = DEMO_PLACES[(item.id % DEMO_PLACES.size).toInt()]
            val jLat = ((item.id * 7919 % 1000) - 500) / 1000.0 * 0.08
            val jLon = ((item.id * 104729 % 1000) - 500) / 1000.0 * 0.12
            MapPoint(item.id, lat + jLat, lon + jLon, item.takenAtUs)
        }
    }

    override fun onThisDay(nowUs: Long, utcOffsetSecs: Int, limit: Int): List<TimelineItem> {
        if (currentAccount() == null || limit <= 0) return emptyList()
        val zone = java.time.ZoneOffset.ofTotalSeconds(utcOffsetSecs.coerceIn(-18 * 3600, 18 * 3600))
        fun date(us: Long) = java.time.Instant.ofEpochSecond(Math.floorDiv(us, 1_000_000L)).atOffset(zone).toLocalDate()
        val today = date(nowUs)
        return items.asSequence()
            .filter { val d = date(it.takenAtUs); d.year < today.year && d.monthValue == today.monthValue && d.dayOfMonth == today.dayOfMonth }
            .take(limit)
            .toList()
    }

    override suspend fun thumbnail(fileId: Long): ByteArray = withContext(ioDispatcher) {
        val item = byId(fileId)
        // Ente thumbnails are at most 720 px; a smaller placeholder keeps the demo light.
        val (w, h) = fit(item.width, item.height, 256)
        PlaceholderPng.render(fileId, w, h)
    }

    override suspend fun download(fileId: Long, destPath: String): String = withContext(ioDispatcher) {
        val item = byId(fileId)
        if (item.kind == FileKind.Video) throw CoreError.Network("Video playback needs a real server")
        delay(latencyMs)
        val (w, h) = fit(item.width, item.height, 1600)
        val dest = File(destPath)
        dest.parentFile?.mkdirs()
        dest.writeBytes(PlaceholderPng.render(fileId, w, h))
        dest.path
    }

    override suspend fun streamPlaylist(fileId: Long): String? {
        byId(fileId)
        return null
    }

    // ---- uploads ----

    override fun enqueueUpload(path: String, collectionId: Long?) {
        synchronized(lock) { pendingUploads++ }
    }

    override suspend fun runUploads(): UploadStatus {
        delay(latencyMs)
        synchronized(lock) { pendingUploads = 0 }
        return uploadStatus()
    }

    override fun uploadStatus(): UploadStatus = synchronized(lock) { UploadStatus(pendingUploads, 0, 0, 0, 0) }

    // ---- helpers ----

    private fun requireAccount(): Account = currentAccount() ?: throw CoreError.NotLoggedIn()

    private fun byId(id: Long): TimelineItem {
        val index = (id - 10_000L).toInt()
        return items.getOrNull(index)?.takeIf { it.id == id } ?: throw CoreError.Storage("No file $id")
    }

    private fun fit(w: Int, h: Int, maxSide: Int): Pair<Int, Int> {
        val scale = maxSide.toFloat() / max(w, h)
        return max(1, (w * scale).roundToInt()) to max(1, (h * scale).roundToInt())
    }

    private fun normalize(address: String): String {
        val a = address.trim().trimEnd('/')
        return if (a.startsWith("http://", true) || a.startsWith("https://", true)) a else "http://$a"
    }

    companion object {
        /** Demo map places: Lisbon, Paris, Berlin, Reykjavik, Kyoto. */
        private val DEMO_PLACES = listOf(38.72 to -9.14, 48.86 to 2.35, 52.52 to 13.40, 64.15 to -21.94, 35.01 to 135.77)
        const val DEMO_CODE = "123456"
        private const val MINUTE_US = 60_000_000L
        private const val HOUR_US = 60 * MINUTE_US
    }
}
