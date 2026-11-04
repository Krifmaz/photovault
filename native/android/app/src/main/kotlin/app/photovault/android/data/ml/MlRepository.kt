package app.photovault.android.data.ml

import app.photovault.android.data.TimelineItem

/*
 * Search, people and memories (contracts/photovault-core.md, "v1.2: search, faces, memories").
 * These mirror the UniFFI records one to one with UI-friendly Kotlin types, like CoreModels.kt.
 * RealMlRepository (src/realCore) adapts the generated `app.photovault.core.Core`; FakeMlRepository
 * (data/ml/fake) serves the demo library. Errors are thrown as app.photovault.android.data.CoreError.
 */

/** `MlStatus` record. */
data class MlStatus(
    val enabled: Boolean,
    val inferenceAvailable: Boolean,
    val modelsReady: Boolean,
    val modelBytesDone: Long,
    val modelBytesTotal: Long,
    val filesTotal: Long,
    val filesIndexed: Long,
    val filesPending: Long,
    val running: Boolean,
    val lastError: String?,
)

/** `SearchKind` enum. */
enum class SearchKind { Magic, Person, Album, Date, Place, FileName, Caption, FileType }

/** `SearchSuggestion` record. [key] is opaque: pass it back to [MlRepository.searchResults]. */
data class SearchSuggestion(
    val kind: SearchKind,
    val key: String,
    val label: String,
    val count: Long,
    val coverFileId: Long?,
    val coverFaceId: String?,
)

/** `FaceRef` record; the box is normalised to [0, 1]. */
data class FaceRef(
    val faceId: String,
    val fileId: Long,
    val x: Float,
    val y: Float,
    val width: Float,
    val height: Float,
    val score: Float,
    val blur: Float,
)

/** `Person` record. [id] is a cgroup id, or `cluster:<id>` for an unnamed cluster ([name] null). */
data class Person(
    val id: String,
    val name: String?,
    val isHidden: Boolean,
    val fileCount: Long,
    val cover: FaceRef?,
    val birthDate: String?,
)

/** `FileFace` record. */
data class FileFace(val face: FaceRef, val personId: String?, val personName: String?)

/** `MemoryKind` enum. */
enum class MemoryKind { OnThisDay, People, Trip, Time, Clip, Filler }

/** `Memory` record. */
data class Memory(
    val id: String,
    val kind: MemoryKind,
    val title: String,
    val subtitle: String?,
    val items: List<TimelineItem>,
    val coverFileId: Long,
    val seen: Boolean,
    val personId: String?,
)

/** The search / people / memories part of photovault-core. */
interface MlRepository {
    // ML control
    fun mlStatus(): MlStatus
    suspend fun setMlEnabled(enabled: Boolean): MlStatus
    suspend fun syncMl(onStatus: ((MlStatus) -> Unit)?): MlStatus
    suspend fun runMlIndexing(maxFiles: Int, onStatus: ((MlStatus) -> Unit)?): MlStatus

    // search
    fun searchSuggestions(query: String, limit: Int): List<SearchSuggestion>
    fun searchDiscovery(): List<SearchSuggestion>
    suspend fun searchResults(kind: SearchKind, key: String): List<TimelineItem>
    suspend fun magicSearch(query: String, limit: Int): List<TimelineItem>

    // people
    fun people(includeHidden: Boolean, minFiles: Int): List<Person>
    fun person(personId: String): Person?
    fun personFiles(personId: String): List<TimelineItem>
    fun fileFaces(fileId: Long): List<FileFace>
    fun personSuggestions(personId: String): List<Person>
    suspend fun faceThumbnail(faceId: String): ByteArray
    suspend fun namePerson(personId: String, name: String): Person
    suspend fun mergePeople(intoId: String, fromId: String): Person
    suspend fun answerSuggestion(personId: String, clusterId: String, accept: Boolean)
    suspend fun setPersonHidden(personId: String, hidden: Boolean)
    suspend fun setPersonCover(personId: String, faceId: String)
    suspend fun setPersonBirthDate(personId: String, birthDate: String?)
    suspend fun removeFacesFromPerson(personId: String, faceIds: List<String>)
    suspend fun deletePerson(personId: String)

    // memories
    fun memories(nowUs: Long, utcOffsetSecs: Int): List<Memory>
    fun markMemorySeen(memoryId: String)
}
