package app.photovault.android.data.albums

import app.photovault.android.data.PhotoCollection
import app.photovault.android.data.TimelineItem

/*
 * Albums, sharing, hidden albums and trash: the app's view of the photovault-core additions in
 * core/src/albums.rs (contracts/photovault-core.md, "Albums, sharing and trash"). Mirrors the UniFFI
 * records one to one, with Kotlin types (Long instead of ULong, ...).
 */

/** `ShareRole` enum: Ente's collection participant roles. */
enum class ShareRole { Viewer, Collaborator, Admin, Owner, Unknown }

/** `Sharee` record: someone an album is shared with. */
data class Sharee(val userId: Long, val email: String, val role: ShareRole)

/**
 * `PublicLink` record. [url] is the full link, the album key already in its `#` fragment (base58, as Ente's
 * apps write it), so it opens in the server's public album page. [validTillUs] 0 = never expires,
 * [deviceLimit] 0 = unlimited.
 */
data class PublicLink(
    val url: String,
    val validTillUs: Long,
    val deviceLimit: Int,
    val enableDownload: Boolean,
    val enableCollect: Boolean,
    val enableJoin: Boolean,
    val passwordEnabled: Boolean,
)

/** `PublicLinkUpdate` record: only non-null fields change. [password] "" removes the password. */
data class PublicLinkUpdate(
    val validTillUs: Long? = null,
    val deviceLimit: Int? = null,
    val enableDownload: Boolean? = null,
    val enableCollect: Boolean? = null,
    val enableJoin: Boolean? = null,
    val password: String? = null,
)

/**
 * `AlbumDetails` record. [myRole] is Owner for the user's own albums. [isDefaultHidden] marks Ente's
 * "hidden photos" collection (individually hidden files), which is not an album the user created.
 */
data class AlbumDetails(
    val collection: PhotoCollection,
    val owned: Boolean,
    val ownerEmail: String,
    val myRole: ShareRole,
    val hidden: Boolean,
    val archived: Boolean,
    val isDefaultHidden: Boolean,
    val sharees: List<Sharee>,
    val publicLink: PublicLink?,
) {
    val id: Long get() = collection.id
    val canAdd: Boolean get() = myRole == ShareRole.Owner || myRole == ShareRole.Collaborator || myRole == ShareRole.Admin
    val canManage: Boolean get() = owned
}

/** `TrashEntry` record: a trashed file and when the server deletes it for good. */
data class TrashEntry(val item: TimelineItem, val deleteByUs: Long)
