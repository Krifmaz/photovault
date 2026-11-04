package app.photovault.android.data.albums

import app.photovault.android.data.CoreError
import app.photovault.android.data.PhotoCollection
import app.photovault.android.data.TimelineItem
import app.photovault.android.data.fake.FakeCore
import kotlinx.coroutines.delay

/**
 * In-memory albums, sharing and trash over [FakeCore]'s demo library, so the demo APK (and the unit tests)
 * can reach every screen: the demo albums, one shared with two people and a public link, one shared with
 * the user by someone else, a hidden album, a few individually hidden photos and some trash.
 *
 * Demo rules: sharing with an address containing "unknown" fails like a server would for someone without
 * an account; everything else succeeds.
 */
class FakeAlbums(
    private val core: FakeCore,
    private val latencyMs: Long = core.demoLatencyMs / 2,
) : AlbumsRepository {

    override val isDemo: Boolean get() = true

    private class Album(
        val id: Long,
        var name: String,
        val kind: String,
        val owned: Boolean,
        val ownerEmail: String,
        val myRole: ShareRole,
        var hidden: Boolean = false,
        val defaultHidden: Boolean = false,
        val sharees: MutableList<Sharee> = ArrayList(),
        var link: PublicLink? = null,
        /** File ids, kept newest first. */
        val files: MutableList<Long> = ArrayList(),
        var updatedAtUs: Long,
    )

    private val lock = Any()
    private val byId: Map<Long, TimelineItem> = core.demoItems.associateBy { it.id }
    private val albums = LinkedHashMap<Long, Album>()
    private val trash = ArrayList<TrashEntry>()
    private var nextId = 100L
    private val nowUs = core.demoNowUs

    init {
        val items = core.demoItems
        for (c in core.demoCollections) {
            val sharedWithMe = c.name == "Hikes"
            albums[c.id] = Album(
                id = c.id,
                name = c.name,
                kind = c.kind,
                owned = !sharedWithMe,
                ownerEmail = if (sharedWithMe) "sam@example.com" else "",
                myRole = if (sharedWithMe) ShareRole.Collaborator else ShareRole.Owner,
                files = items.filter { it.collectionId == c.id }.mapTo(ArrayList()) { it.id },
                updatedAtUs = c.updatedAtUs,
            )
        }
        albums.values.firstOrNull { it.name == "Family" }?.apply {
            sharees += Sharee(2, "alex@example.com", ShareRole.Collaborator)
            sharees += Sharee(3, "robin@example.com", ShareRole.Viewer)
            link = PublicLink(
                url = "https://photos.example.com/?t=Fam1lyL1nk#4mBNsWzQ1v8qbN2xY7dHkT9pRc3fGjLu",
                validTillUs = 0,
                deviceLimit = 0,
                enableDownload = true,
                enableCollect = false,
                enableJoin = true,
                passwordEnabled = false,
            )
        }
        albums.values.firstOrNull { it.name == "Hikes" }?.apply {
            sharees += Sharee(1, "me@example.com", ShareRole.Collaborator)
        }
        val privateId = newId()
        albums[privateId] = Album(
            id = privateId, name = "Private", kind = "album", owned = true, ownerEmail = "", myRole = ShareRole.Owner,
            hidden = true, files = items.filterIndexed { i, _ -> i % 37 == 3 }.mapTo(ArrayList()) { it.id },
            updatedAtUs = nowUs,
        )
        val hiddenPhotos = newId()
        albums[hiddenPhotos] = Album(
            id = hiddenPhotos, name = "Hidden", kind = "album", owned = true, ownerEmail = "", myRole = ShareRole.Owner,
            hidden = true, defaultHidden = true,
            files = items.filterIndexed { i, _ -> i % 53 == 7 }.mapTo(ArrayList()) { it.id },
            updatedAtUs = nowUs,
        )
        // Trash: the oldest few photos, trashed over the last weeks (30 days before the server deletes them).
        items.takeLast(9).forEachIndexed { i, item ->
            val trashedAt = nowUs - (i * 3L + 1) * DAY_US
            trash += TrashEntry(item.copy(collectionId = 1), trashedAt + 30 * DAY_US)
        }
    }

    private fun newId(): Long = nextId++

    private fun requireAccount() {
        if (core.currentAccount() == null) throw CoreError.NotLoggedIn()
    }

    private fun album(id: Long): Album = albums[id] ?: throw CoreError.Server(404, "No album $id")

    private fun owned(id: Long): Album = album(id).also { if (!it.owned) throw CoreError.Server(403, "Not your album") }

    private fun Album.sortFiles() {
        files.sortByDescending { byId[it]?.takenAtUs ?: 0L }
    }

    private fun Album.details(): AlbumDetails {
        val cover = files.firstOrNull()
        return AlbumDetails(
            collection = PhotoCollection(
                id = id,
                name = name,
                kind = kind,
                coverFileId = cover,
                count = files.size.toLong(),
                updatedAtUs = updatedAtUs,
            ),
            owned = owned,
            ownerEmail = ownerEmail,
            myRole = myRole,
            hidden = hidden,
            archived = false,
            isDefaultHidden = defaultHidden,
            sharees = sharees.toList(),
            publicLink = link,
        )
    }

    private fun Album.newest(): Long = files.firstOrNull()?.let { byId[it]?.takenAtUs } ?: updatedAtUs

    // ---- albums ----

    override fun albums(): List<AlbumDetails> = synchronized(lock) {
        if (core.currentAccount() == null) emptyList()
        else albums.values.filter { !it.hidden }.sortedByDescending { it.newest() }.map { it.details() }
    }

    override fun hiddenAlbums(): List<AlbumDetails> = synchronized(lock) {
        if (core.currentAccount() == null) emptyList()
        else albums.values.filter { it.hidden }.sortedWith(compareBy<Album> { !it.defaultHidden }.thenByDescending { it.newest() })
            .map { it.details() }
    }

    override fun albumDetails(collectionId: Long): AlbumDetails? = synchronized(lock) {
        if (core.currentAccount() == null) null else albums[collectionId]?.details()
    }

    override fun albumCount(collectionId: Long): Long = synchronized(lock) {
        if (core.currentAccount() == null) 0 else albums[collectionId]?.files?.size?.toLong() ?: 0
    }

    override fun albumPage(collectionId: Long, offset: Long, limit: Int): List<TimelineItem> {
        synchronized(lock) {
            if (core.currentAccount() == null) return emptyList()
            val files = albums[collectionId]?.files ?: return emptyList()
            if (offset >= files.size || limit <= 0) return emptyList()
            return files.subList(offset.toInt(), minOf(files.size, offset.toInt() + limit))
                .mapNotNull { byId[it]?.copy(collectionId = collectionId) }
        }
    }

    override suspend fun createAlbum(name: String): PhotoCollection {
        delay(latencyMs)
        requireAccount()
        val clean = name.trim()
        if (clean.isEmpty()) throw CoreError.Server(400, "An album needs a name")
        return synchronized(lock) {
            val a = Album(newId(), clean, "album", owned = true, ownerEmail = "", myRole = ShareRole.Owner, updatedAtUs = nowUs)
            albums[a.id] = a
            a.details().collection
        }
    }

    override suspend fun renameAlbum(collectionId: Long, name: String) {
        delay(latencyMs)
        requireAccount()
        val clean = name.trim()
        if (clean.isEmpty()) throw CoreError.Server(400, "An album needs a name")
        synchronized(lock) { owned(collectionId).name = clean }
    }

    override suspend fun deleteAlbum(collectionId: Long, keepFiles: Boolean) {
        delay(latencyMs)
        requireAccount()
        synchronized(lock) {
            val a = owned(collectionId)
            albums.remove(collectionId)
            val elsewhere = albums.values.flatMapTo(HashSet()) { it.files }
            val only = a.files.filter { it !in elsewhere }
            if (keepFiles) {
                // like core: into Uncategorized (the demo has none, so the first own album stands in)
                albums.values.firstOrNull { it.owned && !it.hidden }?.let { u -> u.files += only; u.sortFiles() }
            } else {
                only.forEach { id -> byId[id]?.let { trash.add(0, TrashEntry(it, nowUs + 30 * DAY_US)) } }
            }
        }
    }

    override suspend fun addToAlbum(collectionId: Long, fileIds: List<Long>) {
        delay(latencyMs)
        requireAccount()
        synchronized(lock) {
            val a = album(collectionId)
            if (!a.owned && a.myRole != ShareRole.Collaborator && a.myRole != ShareRole.Admin) throw CoreError.Server(403, "Viewers can't add photos")
            fileIds.filter { it in byId && it !in a.files }.forEach { a.files += it }
            a.sortFiles()
        }
    }

    override suspend fun removeFromAlbum(collectionId: Long, fileIds: List<Long>) {
        delay(latencyMs)
        requireAccount()
        synchronized(lock) { album(collectionId).files.removeAll(fileIds.toSet()) }
    }

    override suspend fun setAlbumHidden(collectionId: Long, hidden: Boolean) {
        delay(latencyMs)
        requireAccount()
        synchronized(lock) {
            val a = owned(collectionId)
            if (a.defaultHidden) throw CoreError.Server(400, "This collection is always hidden")
            a.hidden = hidden
        }
    }

    private fun defaultHidden(): Album = albums.values.first { it.defaultHidden }

    override suspend fun hideFiles(fileIds: List<Long>) {
        delay(latencyMs)
        requireAccount()
        synchronized(lock) {
            val ids = fileIds.toSet()
            albums.values.filter { it.owned && !it.defaultHidden }.forEach { it.files.removeAll(ids) }
            val h = defaultHidden()
            ids.filter { it in byId && it !in h.files }.forEach { h.files += it }
            h.sortFiles()
        }
    }

    override suspend fun unhideFiles(fileIds: List<Long>, collectionId: Long?) {
        delay(latencyMs)
        requireAccount()
        synchronized(lock) {
            val ids = fileIds.toSet()
            defaultHidden().files.removeAll(ids)
            val target = collectionId?.let { owned(it) } ?: albums.values.first { it.owned && !it.hidden }
            ids.filter { it in byId && it !in target.files }.forEach { target.files += it }
            target.sortFiles()
        }
    }

    // ---- sharing ----

    override suspend fun shareAlbum(collectionId: Long, email: String, role: ShareRole): List<Sharee> {
        delay(latencyMs)
        requireAccount()
        val e = email.trim().lowercase()
        if (!e.contains('@')) throw CoreError.Server(400, "Enter an email address")
        if ("unknown" in e) throw CoreError.Server(404, "$e has no account on this server")
        return synchronized(lock) {
            val a = owned(collectionId)
            a.sharees.removeAll { it.email == e }
            a.sharees += Sharee(nextId++, e, role)
            a.sharees.toList()
        }
    }

    override suspend fun unshareAlbum(collectionId: Long, email: String): List<Sharee> {
        delay(latencyMs)
        requireAccount()
        return synchronized(lock) {
            val a = owned(collectionId)
            a.sharees.removeAll { it.email.equals(email, ignoreCase = true) }
            a.sharees.toList()
        }
    }

    override suspend fun leaveAlbum(collectionId: Long) {
        delay(latencyMs)
        requireAccount()
        synchronized(lock) {
            val a = album(collectionId)
            if (a.owned) throw CoreError.Server(400, "You own this album")
            albums.remove(collectionId)
        }
    }

    override suspend fun createPublicLink(collectionId: Long): PublicLink {
        delay(latencyMs)
        requireAccount()
        return synchronized(lock) {
            val a = owned(collectionId)
            a.link ?: PublicLink(
                url = "https://photos.example.com/?t=Demo${collectionId}Link#7Yq${collectionId}hT2mXcVb9KdPzR4sWnLgE",
                validTillUs = 0,
                deviceLimit = 0,
                enableDownload = true,
                enableCollect = false,
                enableJoin = true,
                passwordEnabled = false,
            ).also { a.link = it }
        }
    }

    override suspend fun updatePublicLink(collectionId: Long, update: PublicLinkUpdate): PublicLink {
        delay(latencyMs)
        requireAccount()
        return synchronized(lock) {
            val a = owned(collectionId)
            val l = a.link ?: throw CoreError.Server(404, "This album has no link")
            l.copy(
                validTillUs = update.validTillUs ?: l.validTillUs,
                deviceLimit = update.deviceLimit ?: l.deviceLimit,
                enableDownload = update.enableDownload ?: l.enableDownload,
                enableCollect = update.enableCollect ?: l.enableCollect,
                enableJoin = update.enableJoin ?: l.enableJoin,
                passwordEnabled = update.password?.isNotEmpty() ?: l.passwordEnabled,
            ).also { a.link = it }
        }
    }

    override suspend fun deletePublicLink(collectionId: Long) {
        delay(latencyMs)
        requireAccount()
        synchronized(lock) { owned(collectionId).link = null }
    }

    // ---- trash ----

    override suspend fun syncTrash(): Long {
        delay(latencyMs)
        requireAccount()
        return trashCount()
    }

    override fun trashCount(): Long = synchronized(lock) { if (core.currentAccount() == null) 0 else trash.size.toLong() }

    override fun trashPage(offset: Long, limit: Int): List<TrashEntry> = synchronized(lock) {
        if (core.currentAccount() == null || offset >= trash.size || limit <= 0) emptyList()
        else trash.subList(offset.toInt(), minOf(trash.size, offset.toInt() + limit)).toList()
    }

    override suspend fun moveToTrash(fileIds: List<Long>) {
        delay(latencyMs)
        requireAccount()
        synchronized(lock) {
            val ids = fileIds.toSet()
            albums.values.filter { it.owned }.forEach { it.files.removeAll(ids) }
            ids.mapNotNull { byId[it] }.sortedByDescending { it.takenAtUs }.asReversed()
                .forEach { trash.add(0, TrashEntry(it, nowUs + 30 * DAY_US)) }
        }
    }

    override suspend fun restoreFromTrash(fileIds: List<Long>, collectionId: Long?) {
        delay(latencyMs)
        requireAccount()
        synchronized(lock) {
            val ids = fileIds.toSet()
            val target = collectionId?.let { owned(it) } ?: albums.values.first { it.owned && !it.hidden }
            val restored = trash.filter { it.item.id in ids }
            trash.removeAll(restored.toSet())
            restored.forEach { if (it.item.id !in target.files) target.files += it.item.id }
            target.sortFiles()
        }
    }

    override suspend fun deleteFromTrash(fileIds: List<Long>) {
        delay(latencyMs)
        requireAccount()
        synchronized(lock) {
            val ids = fileIds.toSet()
            trash.removeAll { it.item.id in ids }
        }
    }

    override suspend fun emptyTrash() {
        delay(latencyMs)
        requireAccount()
        synchronized(lock) { trash.clear() }
    }

    companion object {
        const val DAY_US = 86_400_000_000L
    }
}
