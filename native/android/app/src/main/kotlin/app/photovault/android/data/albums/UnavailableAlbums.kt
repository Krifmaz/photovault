package app.photovault.android.data.albums

import app.photovault.android.data.CoreError
import app.photovault.android.data.PhotoCollection
import app.photovault.android.data.TimelineItem

/** With [app.photovault.android.data.UnavailableCore]: nothing to show, every change fails. */
object UnavailableAlbums : AlbumsRepository {
    private fun fail(): Nothing = throw CoreError.NotLoggedIn()

    override fun albums(): List<AlbumDetails> = emptyList()
    override fun hiddenAlbums(): List<AlbumDetails> = emptyList()
    override fun albumDetails(collectionId: Long): AlbumDetails? = null
    override fun albumCount(collectionId: Long): Long = 0
    override fun albumPage(collectionId: Long, offset: Long, limit: Int): List<TimelineItem> = emptyList()
    override suspend fun createAlbum(name: String): PhotoCollection = fail()
    override suspend fun renameAlbum(collectionId: Long, name: String): Unit = fail()
    override suspend fun deleteAlbum(collectionId: Long, keepFiles: Boolean): Unit = fail()
    override suspend fun addToAlbum(collectionId: Long, fileIds: List<Long>): Unit = fail()
    override suspend fun removeFromAlbum(collectionId: Long, fileIds: List<Long>): Unit = fail()
    override suspend fun setAlbumHidden(collectionId: Long, hidden: Boolean): Unit = fail()
    override suspend fun hideFiles(fileIds: List<Long>): Unit = fail()
    override suspend fun unhideFiles(fileIds: List<Long>, collectionId: Long?): Unit = fail()
    override suspend fun shareAlbum(collectionId: Long, email: String, role: ShareRole): List<Sharee> = fail()
    override suspend fun unshareAlbum(collectionId: Long, email: String): List<Sharee> = fail()
    override suspend fun leaveAlbum(collectionId: Long): Unit = fail()
    override suspend fun createPublicLink(collectionId: Long): PublicLink = fail()
    override suspend fun updatePublicLink(collectionId: Long, update: PublicLinkUpdate): PublicLink = fail()
    override suspend fun deletePublicLink(collectionId: Long): Unit = fail()
    override suspend fun syncTrash(): Long = fail()
    override fun trashCount(): Long = 0
    override fun trashPage(offset: Long, limit: Int): List<TrashEntry> = emptyList()
    override suspend fun moveToTrash(fileIds: List<Long>): Unit = fail()
    override suspend fun restoreFromTrash(fileIds: List<Long>, collectionId: Long?): Unit = fail()
    override suspend fun deleteFromTrash(fileIds: List<Long>): Unit = fail()
    override suspend fun emptyTrash(): Unit = fail()
}
