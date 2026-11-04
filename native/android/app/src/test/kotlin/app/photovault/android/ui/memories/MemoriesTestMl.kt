package app.photovault.android.ui.memories

import app.photovault.android.data.CoreError
import app.photovault.android.data.FileKind
import app.photovault.android.data.TimelineItem
import app.photovault.android.data.ml.FileFace
import app.photovault.android.data.ml.Memory
import app.photovault.android.data.ml.MemoryKind
import app.photovault.android.data.ml.MlRepository
import app.photovault.android.data.ml.MlStatus
import app.photovault.android.data.ml.Person
import app.photovault.android.data.ml.SearchKind
import app.photovault.android.data.ml.SearchSuggestion

/** A small recording MlRepository for the memories and ML settings tests (only what they use does anything). */
internal class MemoriesTestMl(
    var memoryList: List<Memory> = emptyList(),
    var status: MlStatus = mlStatus(),
) : MlRepository {
    val memoriesCalls = mutableListOf<Pair<Long, Int>>()
    val seen = mutableListOf<String>()
    val enabledCalls = mutableListOf<Boolean>()
    var failSetEnabled: CoreError? = null
    var failMemories: CoreError? = null

    override fun mlStatus(): MlStatus = status

    override suspend fun setMlEnabled(enabled: Boolean): MlStatus {
        enabledCalls += enabled
        failSetEnabled?.let { throw it }
        status = status.copy(enabled = enabled)
        return status
    }

    override suspend fun syncMl(onStatus: ((MlStatus) -> Unit)?): MlStatus = status
    override suspend fun runMlIndexing(maxFiles: Int, onStatus: ((MlStatus) -> Unit)?): MlStatus = status

    override fun searchSuggestions(query: String, limit: Int): List<SearchSuggestion> = emptyList()
    override fun searchDiscovery(): List<SearchSuggestion> = emptyList()
    override suspend fun searchResults(kind: SearchKind, key: String): List<TimelineItem> = emptyList()
    override suspend fun magicSearch(query: String, limit: Int): List<TimelineItem> = emptyList()

    override fun people(includeHidden: Boolean, minFiles: Int): List<Person> = emptyList()
    override fun person(personId: String): Person? = null
    override fun personFiles(personId: String): List<TimelineItem> = emptyList()
    override fun fileFaces(fileId: Long): List<FileFace> = emptyList()
    override fun personSuggestions(personId: String): List<Person> = emptyList()
    override suspend fun faceThumbnail(faceId: String): ByteArray = ByteArray(0)
    override suspend fun namePerson(personId: String, name: String): Person = throw CoreError.Storage("unused")
    override suspend fun mergePeople(intoId: String, fromId: String): Person = throw CoreError.Storage("unused")
    override suspend fun answerSuggestion(personId: String, clusterId: String, accept: Boolean) = Unit
    override suspend fun setPersonHidden(personId: String, hidden: Boolean) = Unit
    override suspend fun setPersonCover(personId: String, faceId: String) = Unit
    override suspend fun setPersonBirthDate(personId: String, birthDate: String?) = Unit
    override suspend fun removeFacesFromPerson(personId: String, faceIds: List<String>) = Unit
    override suspend fun deletePerson(personId: String) = Unit

    override fun memories(nowUs: Long, utcOffsetSecs: Int): List<Memory> {
        memoriesCalls += nowUs to utcOffsetSecs
        failMemories?.let { throw it }
        return memoryList
    }

    override fun markMemorySeen(memoryId: String) {
        seen += memoryId
        memoryList = memoryList.map { if (it.id == memoryId) it.copy(seen = true) else it }
    }
}

internal fun mlStatus(
    enabled: Boolean = false,
    inferenceAvailable: Boolean = true,
    modelsReady: Boolean = false,
    modelBytesDone: Long = 0,
    modelBytesTotal: Long = 0,
    filesTotal: Long = 0,
    filesIndexed: Long = 0,
    filesPending: Long = 0,
    running: Boolean = false,
    lastError: String? = null,
) = MlStatus(enabled, inferenceAvailable, modelsReady, modelBytesDone, modelBytesTotal, filesTotal, filesIndexed, filesPending, running, lastError)

internal fun item(id: Long, kind: FileKind = FileKind.Image): TimelineItem =
    TimelineItem(id, 1, kind, 1_700_000_000_000_000L - id * 86_400_000_000L, 4000, 3000, null, false, "IMG_$id.jpg")

internal fun memory(id: String, seen: Boolean = false, items: List<TimelineItem> = listOf(item(1), item(2), item(3))): Memory =
    Memory(id, MemoryKind.OnThisDay, "Title $id", null, items, items.firstOrNull()?.id ?: 0, seen, null)
