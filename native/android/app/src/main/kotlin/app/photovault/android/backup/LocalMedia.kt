package app.photovault.android.backup

import android.Manifest
import android.content.ContentResolver
import android.content.Context
import android.content.pm.PackageManager
import android.database.Cursor
import android.net.Uri
import android.os.Build
import android.provider.MediaStore
import androidx.compose.runtime.Immutable
import java.io.File

/** A device folder (MediaStore bucket) that holds photos or videos. */
@Immutable
data class LocalFolder(val id: String, val name: String, val count: Int, val isCamera: Boolean)

/** One photo or video on the device. */
data class LocalMedia(
    /** Stable per device: volume, kind and MediaStore id. */
    val key: String,
    /** Filesystem path photovault-core reads (null when MediaStore has none). */
    val path: String?,
    val folderId: String,
    val folderName: String,
    /** MediaStore DATE_MODIFIED (seconds) and SIZE: a change means the file was edited. */
    val modified: Long,
    val size: Long,
    val isVideo: Boolean,
)

/** How much of the media library the app may read. Partial = Android 14's "selected photos only". */
enum class MediaAccess { None, Partial, Full }

/** The device's media library. Every call does disk work: call off the main thread. */
interface MediaSource {
    fun access(): MediaAccess
    fun folders(): List<LocalFolder>
    /** The photos (and [includeVideos] videos) in [folderIds], newest first. */
    fun media(folderIds: Set<String>, includeVideos: Boolean): List<LocalMedia>
    fun readable(path: String): Boolean
}

/** The runtime permissions backup asks for on this Android version. */
fun backupPermissions(sdk: Int = Build.VERSION.SDK_INT): Array<String> = buildList {
    when {
        sdk >= 34 -> addAll(listOf(Manifest.permission.READ_MEDIA_IMAGES, Manifest.permission.READ_MEDIA_VIDEO, Manifest.permission.READ_MEDIA_VISUAL_USER_SELECTED))
        sdk >= 33 -> addAll(listOf(Manifest.permission.READ_MEDIA_IMAGES, Manifest.permission.READ_MEDIA_VIDEO))
        else -> add(Manifest.permission.READ_EXTERNAL_STORAGE)
    }
    // Without it Android strips the GPS position from the photos the app reads (Android 10+).
    if (sdk >= 29) add(Manifest.permission.ACCESS_MEDIA_LOCATION)
    // The progress notification while uploading.
    if (sdk >= 33) add(Manifest.permission.POST_NOTIFICATIONS)
}.toTypedArray()

/** Folders whose files a phone's camera writes: DCIM/Camera on every volume. */
internal fun isCameraFolder(name: String, relativeOrPath: String?): Boolean {
    val p = relativeOrPath?.replace('\\', '/')?.trimEnd('/')?.lowercase()
    return p != null && (p == "dcim/camera" || p.endsWith("/dcim/camera")) ||
        (p == null && name.equals("Camera", ignoreCase = true))
}

class MediaStoreSource(private val context: Context) : MediaSource {
    private val resolver: ContentResolver get() = context.contentResolver

    private fun granted(permission: String) =
        context.checkSelfPermission(permission) == PackageManager.PERMISSION_GRANTED

    override fun access(): MediaAccess = when {
        Build.VERSION.SDK_INT >= 33 && granted(Manifest.permission.READ_MEDIA_IMAGES) -> MediaAccess.Full
        Build.VERSION.SDK_INT >= 34 && granted(Manifest.permission.READ_MEDIA_VISUAL_USER_SELECTED) -> MediaAccess.Partial
        Build.VERSION.SDK_INT < 33 && granted(Manifest.permission.READ_EXTERNAL_STORAGE) -> MediaAccess.Full
        else -> MediaAccess.None
    }

    private fun collections(videos: Boolean): List<Pair<Uri, Boolean>> = buildList {
        if (Build.VERSION.SDK_INT >= 29) {
            add(MediaStore.Images.Media.getContentUri(MediaStore.VOLUME_EXTERNAL) to false)
            if (videos) add(MediaStore.Video.Media.getContentUri(MediaStore.VOLUME_EXTERNAL) to true)
        } else {
            add(MediaStore.Images.Media.EXTERNAL_CONTENT_URI to false)
            if (videos) add(MediaStore.Video.Media.EXTERNAL_CONTENT_URI to true)
        }
    }

    override fun folders(): List<LocalFolder> {
        if (access() == MediaAccess.None) return emptyList()
        class Acc(val name: String, var count: Int, var camera: Boolean)
        val byId = LinkedHashMap<String, Acc>()
        val where = if (Build.VERSION.SDK_INT >= 29) "$C_PENDING = 0" else null
        for ((uri, _) in collections(videos = true)) {
            val locate = if (Build.VERSION.SDK_INT >= 29) C_RELATIVE_PATH else C_DATA
            query(uri, arrayOf(C_BUCKET_ID, C_BUCKET_NAME, locate), where, null, null) { c ->
                val iId = c.getColumnIndexOrThrow(C_BUCKET_ID)
                val iName = c.getColumnIndexOrThrow(C_BUCKET_NAME)
                val iLoc = c.getColumnIndexOrThrow(locate)
                while (c.moveToNext()) {
                    val id = c.getString(iId) ?: continue
                    val name = c.getString(iName) ?: ""
                    val loc = c.getString(iLoc)?.let { if (locate == C_DATA) File(it).parent else it }
                    val acc = byId.getOrPut(id) { Acc(name, 0, false) }
                    acc.count++
                    if (!acc.camera && isCameraFolder(name, loc)) acc.camera = true
                }
            }
        }
        return byId.map { (id, a) -> LocalFolder(id, a.name.ifBlank { id }, a.count, a.camera) }
            .sortedWith(compareByDescending<LocalFolder> { it.isCamera }.thenBy { it.name.lowercase() })
    }

    override fun media(folderIds: Set<String>, includeVideos: Boolean): List<LocalMedia> {
        if (folderIds.isEmpty() || access() == MediaAccess.None) return emptyList()
        val out = ArrayList<LocalMedia>()
        val ids = folderIds.toList()
        // SQLite's bound-parameter limit is far above any folder count, but stay well under it.
        for (chunk in ids.chunked(200)) {
            val placeholders = chunk.joinToString(",") { "?" }
            val where = buildString {
                append("$C_BUCKET_ID IN ($placeholders)")
                if (Build.VERSION.SDK_INT >= 29) append(" AND $C_PENDING = 0")
            }
            for ((uri, video) in collections(includeVideos)) {
                val projection = buildList {
                    addAll(listOf(C_ID, C_BUCKET_ID, C_BUCKET_NAME, C_DATA, C_MODIFIED, C_SIZE))
                    if (Build.VERSION.SDK_INT >= 29) add(C_VOLUME)
                }.toTypedArray()
                query(uri, projection, where, chunk.toTypedArray(), null) { c ->
                    val iId = c.getColumnIndexOrThrow(C_ID)
                    val iBucket = c.getColumnIndexOrThrow(C_BUCKET_ID)
                    val iName = c.getColumnIndexOrThrow(C_BUCKET_NAME)
                    val iData = c.getColumnIndexOrThrow(C_DATA)
                    val iMod = c.getColumnIndexOrThrow(C_MODIFIED)
                    val iSize = c.getColumnIndexOrThrow(C_SIZE)
                    val iVol = c.getColumnIndex(C_VOLUME)
                    while (c.moveToNext()) {
                        val volume = if (iVol >= 0) c.getString(iVol) ?: "external" else "external"
                        val kind = if (video) "v" else "i"
                        out += LocalMedia(
                            key = "$volume:$kind:${c.getLong(iId)}",
                            path = c.getString(iData),
                            folderId = c.getString(iBucket) ?: "",
                            folderName = c.getString(iName) ?: "",
                            modified = c.getLong(iMod),
                            size = c.getLong(iSize),
                            isVideo = video,
                        )
                    }
                }
            }
        }
        // Newest first, so the latest shots are safe soonest. (DATE_MODIFIED is set for every file.)
        out.sortByDescending { it.modified }
        return out
    }

    override fun readable(path: String): Boolean = File(path).let { it.isFile && it.canRead() }

    private inline fun query(
        uri: Uri,
        projection: Array<String>,
        selection: String?,
        args: Array<String>?,
        order: String?,
        block: (Cursor) -> Unit,
    ) {
        try {
            resolver.query(uri, projection, selection, args, order)?.use(block)
        } catch (_: SecurityException) {
            // permission revoked while running: nothing visible
        }
    }

    private companion object {
        // Column names as strings: several moved between MediaColumns and Image/VideoColumns across API levels.
        const val C_ID = "_id"
        const val C_BUCKET_ID = "bucket_id"
        const val C_BUCKET_NAME = "bucket_display_name"
        const val C_DATA = "_data"
        const val C_MODIFIED = "date_modified"
        const val C_SIZE = "_size"
        const val C_VOLUME = "volume_name"
        const val C_RELATIVE_PATH = "relative_path"
        const val C_PENDING = "is_pending"
    }
}
