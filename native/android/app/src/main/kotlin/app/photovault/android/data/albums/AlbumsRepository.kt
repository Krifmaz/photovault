package app.photovault.android.data.albums

import app.photovault.android.data.PhotoCollection
import app.photovault.android.data.TimelineItem

/**
 * Albums, sharing (people and public links), hidden albums and trash, from photovault-core
 * (contracts/photovault-core.md, "Albums, sharing and trash"). Failures are thrown as
 * [app.photovault.android.data.CoreError].
 *
 * Threading as in [app.photovault.android.data.CoreRepository]: `suspend` functions are main-safe; the plain
 * ones read core's in-memory index (< 5 ms) and are called from a background dispatcher. Every mutation
 * updates core's local index before it returns, so re-reading [albums]/[trashPage] shows the change.
 */
interface AlbumsRepository {
    // albums
    fun albums(): List<AlbumDetails>
    fun hiddenAlbums(): List<AlbumDetails>
    fun albumDetails(collectionId: Long): AlbumDetails?
    /** Files in an album (hidden ones included), and a page of them newest first (core's `collection_page`). */
    fun albumCount(collectionId: Long): Long
    fun albumPage(collectionId: Long, offset: Long, limit: Int): List<TimelineItem>
    suspend fun createAlbum(name: String): PhotoCollection
    suspend fun renameAlbum(collectionId: Long, name: String)
    /** [keepFiles]: files only in this album move to Uncategorized; otherwise they go to the trash. */
    suspend fun deleteAlbum(collectionId: Long, keepFiles: Boolean)
    suspend fun addToAlbum(collectionId: Long, fileIds: List<Long>)
    suspend fun removeFromAlbum(collectionId: Long, fileIds: List<Long>)
    suspend fun setAlbumHidden(collectionId: Long, hidden: Boolean)
    suspend fun hideFiles(fileIds: List<Long>)
    /** Moves hidden files back to [collectionId], or Uncategorized when null. */
    suspend fun unhideFiles(fileIds: List<Long>, collectionId: Long?)

    // sharing
    suspend fun shareAlbum(collectionId: Long, email: String, role: ShareRole): List<Sharee>
    suspend fun unshareAlbum(collectionId: Long, email: String): List<Sharee>
    suspend fun leaveAlbum(collectionId: Long)
    suspend fun createPublicLink(collectionId: Long): PublicLink
    suspend fun updatePublicLink(collectionId: Long, update: PublicLinkUpdate): PublicLink
    suspend fun deletePublicLink(collectionId: Long)

    // trash
    /** Pulls the trash from the server; returns the number of trashed files. */
    suspend fun syncTrash(): Long
    fun trashCount(): Long
    /** Most recently trashed first. */
    fun trashPage(offset: Long, limit: Int): List<TrashEntry>
    suspend fun moveToTrash(fileIds: List<Long>)
    /** Restores into [collectionId], or Uncategorized when null. */
    suspend fun restoreFromTrash(fileIds: List<Long>, collectionId: Long?)
    suspend fun deleteFromTrash(fileIds: List<Long>)
    suspend fun emptyTrash()

    /** True for the in-memory demo implementation. */
    val isDemo: Boolean get() = false
}
