package app.photovault.android.data.ml

import app.photovault.android.data.CoreError
import app.photovault.android.data.TimelineItem

/**
 * Search, people and memories when photovault-core could not be loaded (the companion of
 * [app.photovault.android.data.UnavailableCore]): reads are empty, ML is reported off, and every change fails
 * with a [CoreError.Storage] that names the problem.
 */
class UnavailableMlRepository(private val reason: String) : MlRepository {
    private fun fail(): Nothing = throw CoreError.Storage("This app's encryption library could not be loaded ($reason).")

    private val off = MlStatus(
        enabled = false,
        inferenceAvailable = false,
        modelsReady = false,
        modelBytesDone = 0,
        modelBytesTotal = 0,
        filesTotal = 0,
        filesIndexed = 0,
        filesPending = 0,
        running = false,
        lastError = null,
    )

    override fun mlStatus(): MlStatus = off
    override suspend fun setMlEnabled(enabled: Boolean): MlStatus = fail()
    override suspend fun syncMl(onStatus: ((MlStatus) -> Unit)?): MlStatus = off
    override suspend fun runMlIndexing(maxFiles: Int, onStatus: ((MlStatus) -> Unit)?): MlStatus = off

    override fun searchSuggestions(query: String, limit: Int): List<SearchSuggestion> = emptyList()
    override fun searchDiscovery(): List<SearchSuggestion> = emptyList()
    override suspend fun searchResults(kind: SearchKind, key: String): List<TimelineItem> = emptyList()
    override suspend fun magicSearch(query: String, limit: Int): List<TimelineItem> = emptyList()

    override fun people(includeHidden: Boolean, minFiles: Int): List<Person> = emptyList()
    override fun person(personId: String): Person? = null
    override fun personFiles(personId: String): List<TimelineItem> = emptyList()
    override fun fileFaces(fileId: Long): List<FileFace> = emptyList()
    override fun personSuggestions(personId: String): List<Person> = emptyList()
    override suspend fun faceThumbnail(faceId: String): ByteArray = fail()
    override suspend fun namePerson(personId: String, name: String): Person = fail()
    override suspend fun mergePeople(intoId: String, fromId: String): Person = fail()
    override suspend fun answerSuggestion(personId: String, clusterId: String, accept: Boolean): Unit = fail()
    override suspend fun setPersonHidden(personId: String, hidden: Boolean): Unit = fail()
    override suspend fun setPersonCover(personId: String, faceId: String): Unit = fail()
    override suspend fun setPersonBirthDate(personId: String, birthDate: String?): Unit = fail()
    override suspend fun removeFacesFromPerson(personId: String, faceIds: List<String>): Unit = fail()
    override suspend fun deletePerson(personId: String): Unit = fail()

    override fun memories(nowUs: Long, utcOffsetSecs: Int): List<Memory> = emptyList()
    override fun markMemorySeen(memoryId: String) = Unit
}
