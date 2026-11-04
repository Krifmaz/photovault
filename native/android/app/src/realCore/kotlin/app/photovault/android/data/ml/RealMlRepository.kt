package app.photovault.android.data.ml

import app.photovault.android.data.FileKind
import app.photovault.android.data.TimelineItem
import app.photovault.android.data.toApp
import app.photovault.core.Core
import app.photovault.core.CoreException
import app.photovault.core.MlListener
import app.photovault.core.FaceRef as CFaceRef
import app.photovault.core.FileFace as CFileFace
import app.photovault.core.FileKind as CFileKind
import app.photovault.core.Memory as CMemory
import app.photovault.core.MemoryKind as CMemoryKind
import app.photovault.core.MlStatus as CMlStatus
import app.photovault.core.Person as CPerson
import app.photovault.core.SearchKind as CSearchKind
import app.photovault.core.SearchSuggestion as CSearchSuggestion
import app.photovault.core.TimelineItem as CTimelineItem

/**
 * Adapter from the generated `app.photovault.core` search / people / memories API (contract v1.2) to
 * [MlRepository]. Every [CoreException] becomes the matching [app.photovault.android.data.CoreError], as in
 * RealCore.
 *
 * Core's async functions run on core's own tokio runtime, so the `suspend` functions are main-safe. The plain
 * ones read core's in-memory ML caches and the local store; the UI calls them from Dispatchers.IO.
 */
internal class RealMlRepository(private val core: Core) : MlRepository {

    // ---- ML control ----

    override fun mlStatus(): MlStatus = core.mlStatus().toApp()

    override suspend fun setMlEnabled(enabled: Boolean): MlStatus = guard { core.setMlEnabled(enabled).toApp() }

    override suspend fun syncMl(onStatus: ((MlStatus) -> Unit)?): MlStatus = guard { core.syncMl(listener(onStatus)).toApp() }

    override suspend fun runMlIndexing(maxFiles: Int, onStatus: ((MlStatus) -> Unit)?): MlStatus =
        guard { core.runMlIndexing(maxFiles.coerceAtLeast(0).toUInt(), listener(onStatus)).toApp() }

    /** Core calls the listener from its own worker thread; the app's callbacks only update StateFlows. */
    private fun listener(onStatus: ((MlStatus) -> Unit)?): MlListener? = onStatus?.let { cb ->
        object : MlListener {
            override fun onStatus(status: CMlStatus) = cb(status.toApp())
        }
    }

    // ---- search ----

    override fun searchSuggestions(query: String, limit: Int): List<SearchSuggestion> =
        core.searchSuggestions(query, limit.coerceAtLeast(0).toUInt()).map { it.toApp() }

    override fun searchDiscovery(): List<SearchSuggestion> = core.searchDiscovery().map { it.toApp() }

    override suspend fun searchResults(kind: SearchKind, key: String): List<TimelineItem> =
        guard { core.searchResults(kind.toCore(), key).map { it.toApp() } }

    override suspend fun magicSearch(query: String, limit: Int): List<TimelineItem> =
        guard { core.magicSearch(query, limit.coerceAtLeast(0).toUInt()).map { it.toApp() } }

    // ---- people ----

    override fun people(includeHidden: Boolean, minFiles: Int): List<Person> =
        core.people(includeHidden, minFiles.coerceAtLeast(0).toUInt()).map { it.toApp() }

    override fun person(personId: String): Person? = core.person(personId)?.toApp()

    override fun personFiles(personId: String): List<TimelineItem> = core.personFiles(personId).map { it.toApp() }

    override fun fileFaces(fileId: Long): List<FileFace> = core.fileFaces(fileId).map { it.toApp() }

    override fun personSuggestions(personId: String): List<Person> = core.personSuggestions(personId).map { it.toApp() }

    override suspend fun faceThumbnail(faceId: String): ByteArray = guard { core.faceThumbnail(faceId) }

    override suspend fun namePerson(personId: String, name: String): Person = guard { core.namePerson(personId, name).toApp() }

    override suspend fun mergePeople(intoId: String, fromId: String): Person = guard { core.mergePeople(intoId, fromId).toApp() }

    override suspend fun answerSuggestion(personId: String, clusterId: String, accept: Boolean) =
        guard { core.answerSuggestion(personId, clusterId, accept) }

    override suspend fun setPersonHidden(personId: String, hidden: Boolean) = guard { core.setPersonHidden(personId, hidden) }

    override suspend fun setPersonCover(personId: String, faceId: String) = guard { core.setPersonCover(personId, faceId) }

    override suspend fun setPersonBirthDate(personId: String, birthDate: String?) =
        guard { core.setPersonBirthDate(personId, birthDate) }

    override suspend fun removeFacesFromPerson(personId: String, faceIds: List<String>) =
        guard { core.removeFacesFromPerson(personId, faceIds) }

    override suspend fun deletePerson(personId: String) = guard { core.deletePerson(personId) }

    // ---- memories ----

    override fun memories(nowUs: Long, utcOffsetSecs: Int): List<Memory> = core.memories(nowUs, utcOffsetSecs).map { it.toApp() }

    override fun markMemorySeen(memoryId: String) = guardSync { core.markMemorySeen(memoryId) }

    // ---- conversions ----

    private inline fun <T> guardSync(block: () -> T): T =
        try { block() } catch (e: CoreException) { throw e.toApp() }

    private suspend inline fun <T> guard(crossinline block: suspend () -> T): T =
        try { block() } catch (e: CoreException) { throw e.toApp() }

    private fun CMlStatus.toApp() = MlStatus(
        enabled = enabled,
        inferenceAvailable = inferenceAvailable,
        modelsReady = modelsReady,
        modelBytesDone = modelBytesDone.toLong(),
        modelBytesTotal = modelBytesTotal.toLong(),
        filesTotal = filesTotal.toLong(),
        filesIndexed = filesIndexed.toLong(),
        filesPending = filesPending.toLong(),
        running = running,
        lastError = lastError,
    )

    private fun CSearchKind.toApp(): SearchKind = when (this) {
        CSearchKind.MAGIC -> SearchKind.Magic
        CSearchKind.PERSON -> SearchKind.Person
        CSearchKind.ALBUM -> SearchKind.Album
        CSearchKind.DATE -> SearchKind.Date
        CSearchKind.PLACE -> SearchKind.Place
        CSearchKind.FILE_NAME -> SearchKind.FileName
        CSearchKind.CAPTION -> SearchKind.Caption
        CSearchKind.FILE_TYPE -> SearchKind.FileType
    }

    private fun SearchKind.toCore(): CSearchKind = when (this) {
        SearchKind.Magic -> CSearchKind.MAGIC
        SearchKind.Person -> CSearchKind.PERSON
        SearchKind.Album -> CSearchKind.ALBUM
        SearchKind.Date -> CSearchKind.DATE
        SearchKind.Place -> CSearchKind.PLACE
        SearchKind.FileName -> CSearchKind.FILE_NAME
        SearchKind.Caption -> CSearchKind.CAPTION
        SearchKind.FileType -> CSearchKind.FILE_TYPE
    }

    private fun CSearchSuggestion.toApp() =
        SearchSuggestion(kind.toApp(), key, label, count.toLong(), coverFileId, coverFaceId)

    private fun CFaceRef.toApp() = FaceRef(faceId, fileId, x, y, width, height, score, blur)

    private fun CPerson.toApp() = Person(id, name, isHidden, fileCount.toLong(), cover?.toApp(), birthDate)

    private fun CFileFace.toApp() = FileFace(face.toApp(), personId, personName)

    private fun CMemoryKind.toApp(): MemoryKind = when (this) {
        CMemoryKind.ON_THIS_DAY -> MemoryKind.OnThisDay
        CMemoryKind.PEOPLE -> MemoryKind.People
        CMemoryKind.TRIP -> MemoryKind.Trip
        CMemoryKind.TIME -> MemoryKind.Time
        CMemoryKind.CLIP -> MemoryKind.Clip
        CMemoryKind.FILLER -> MemoryKind.Filler
    }

    private fun CMemory.toApp() =
        Memory(id, kind.toApp(), title, subtitle, items.map { it.toApp() }, coverFileId, seen, personId)

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
}
