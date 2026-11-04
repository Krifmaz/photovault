package app.photovault.android.data.albums

import app.photovault.android.data.FileKind
import app.photovault.android.data.PhotoCollection
import app.photovault.android.data.TimelineItem
import app.photovault.android.data.toApp
import app.photovault.core.Core
import app.photovault.core.CoreException
import app.photovault.core.AlbumDetails as CAlbumDetails
import app.photovault.core.Collection as CCollection
import app.photovault.core.FileKind as CFileKind
import app.photovault.core.PublicLink as CPublicLink
import app.photovault.core.PublicLinkUpdate as CPublicLinkUpdate
import app.photovault.core.ShareRole as CShareRole
import app.photovault.core.Sharee as CSharee
import app.photovault.core.TimelineItem as CTimelineItem
import app.photovault.core.TrashEntry as CTrashEntry

/**
 * Adapter from core's albums/sharing/trash API (core/src/albums.rs, contracts/photovault-core.md
 * "Albums, sharing and trash") to the app's types. Every [CoreException] becomes the matching
 * [app.photovault.android.data.CoreError]; async calls run on core's tokio runtime (main-safe).
 */
internal class RealAlbums(private val core: Core) : AlbumsRepository {

    override fun albums(): List<AlbumDetails> = core.albums().map { it.toApp() }

    override fun hiddenAlbums(): List<AlbumDetails> = core.hiddenAlbums().map { it.toApp() }

    override fun albumDetails(collectionId: Long): AlbumDetails? = core.albumDetails(collectionId)?.toApp()

    override fun albumCount(collectionId: Long): Long = core.albumDetails(collectionId)?.collection?.count?.toLong() ?: 0L

    override fun albumPage(collectionId: Long, offset: Long, limit: Int): List<TimelineItem> =
        core.collectionPage(collectionId, offset.coerceAtLeast(0).toULong(), limit.coerceAtLeast(0).toUInt()).map { it.toApp() }

    override suspend fun createAlbum(name: String): PhotoCollection = guard { core.createAlbum(name).toApp() }

    override suspend fun renameAlbum(collectionId: Long, name: String) = guard { core.renameAlbum(collectionId, name) }

    override suspend fun deleteAlbum(collectionId: Long, keepFiles: Boolean) = guard { core.deleteAlbum(collectionId, keepFiles) }

    override suspend fun addToAlbum(collectionId: Long, fileIds: List<Long>) = guard { core.addToAlbum(collectionId, fileIds) }

    override suspend fun removeFromAlbum(collectionId: Long, fileIds: List<Long>) = guard { core.removeFromAlbum(collectionId, fileIds) }

    override suspend fun setAlbumHidden(collectionId: Long, hidden: Boolean) = guard { core.setAlbumHidden(collectionId, hidden) }

    override suspend fun hideFiles(fileIds: List<Long>) = guard { core.hideFiles(fileIds) }

    override suspend fun unhideFiles(fileIds: List<Long>, collectionId: Long?) = guard { core.unhideFiles(fileIds, collectionId) }

    override suspend fun shareAlbum(collectionId: Long, email: String, role: ShareRole): List<Sharee> =
        guard { core.shareAlbum(collectionId, email, role.toCore()).map { it.toApp() } }

    override suspend fun unshareAlbum(collectionId: Long, email: String): List<Sharee> =
        guard { core.unshareAlbum(collectionId, email).map { it.toApp() } }

    override suspend fun leaveAlbum(collectionId: Long) = guard { core.leaveAlbum(collectionId) }

    override suspend fun createPublicLink(collectionId: Long): PublicLink = guard { core.createPublicLink(collectionId).toApp() }

    override suspend fun updatePublicLink(collectionId: Long, update: PublicLinkUpdate): PublicLink = guard {
        core.updatePublicLink(
            collectionId,
            CPublicLinkUpdate(
                validTillUs = update.validTillUs,
                deviceLimit = update.deviceLimit?.coerceAtLeast(0)?.toUInt(),
                enableDownload = update.enableDownload,
                enableCollect = update.enableCollect,
                enableJoin = update.enableJoin,
                password = update.password,
            ),
        ).toApp()
    }

    override suspend fun deletePublicLink(collectionId: Long) = guard { core.deletePublicLink(collectionId) }

    override suspend fun syncTrash(): Long = guard { core.syncTrash().toLong() }

    override fun trashCount(): Long = core.trashCount().toLong()

    override fun trashPage(offset: Long, limit: Int): List<TrashEntry> =
        core.trashPage(offset.coerceAtLeast(0).toULong(), limit.coerceAtLeast(0).toUInt()).map { it.toApp() }

    override suspend fun moveToTrash(fileIds: List<Long>) = guard { core.moveToTrash(fileIds) }

    override suspend fun restoreFromTrash(fileIds: List<Long>, collectionId: Long?) = guard { core.restoreFromTrash(fileIds, collectionId) }

    override suspend fun deleteFromTrash(fileIds: List<Long>) = guard { core.deleteFromTrash(fileIds) }

    override suspend fun emptyTrash() = guard { core.emptyTrash() }

    // ---- conversions ----

    private suspend inline fun <T> guard(crossinline block: suspend () -> T): T =
        try { block() } catch (e: CoreException) { throw e.toApp() }

    private fun CShareRole.toApp(): ShareRole = when (this) {
        CShareRole.VIEWER -> ShareRole.Viewer
        CShareRole.COLLABORATOR -> ShareRole.Collaborator
        CShareRole.ADMIN -> ShareRole.Admin
        CShareRole.OWNER -> ShareRole.Owner
        CShareRole.UNKNOWN -> ShareRole.Unknown
    }

    private fun ShareRole.toCore(): CShareRole = when (this) {
        ShareRole.Viewer -> CShareRole.VIEWER
        ShareRole.Collaborator -> CShareRole.COLLABORATOR
        ShareRole.Admin -> CShareRole.ADMIN
        ShareRole.Owner -> CShareRole.OWNER
        ShareRole.Unknown -> CShareRole.UNKNOWN
    }

    private fun CSharee.toApp() = Sharee(userId, email, role.toApp())

    private fun CPublicLink.toApp() = PublicLink(
        url = url,
        validTillUs = validTillUs,
        deviceLimit = deviceLimit.toInt(),
        enableDownload = enableDownload,
        enableCollect = enableCollect,
        enableJoin = enableJoin,
        passwordEnabled = passwordEnabled,
    )

    private fun CCollection.toApp() = PhotoCollection(id, name, kind, coverFileId, count.toLong(), updatedAtUs)

    private fun CAlbumDetails.toApp() = AlbumDetails(
        collection = collection.toApp(),
        owned = owned,
        ownerEmail = ownerEmail,
        myRole = myRole.toApp(),
        hidden = hidden,
        archived = archived,
        isDefaultHidden = isDefaultHidden,
        sharees = sharees.map { it.toApp() },
        publicLink = publicLink?.toApp(),
    )

    private fun CFileKind.toApp(): FileKind = when (this) {
        CFileKind.IMAGE -> FileKind.Image
        CFileKind.VIDEO -> FileKind.Video
        CFileKind.LIVE_PHOTO -> FileKind.LivePhoto
        CFileKind.UNKNOWN -> FileKind.Unknown
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

    private fun CTrashEntry.toApp() = TrashEntry(item.toApp(), deleteByUs)
}
