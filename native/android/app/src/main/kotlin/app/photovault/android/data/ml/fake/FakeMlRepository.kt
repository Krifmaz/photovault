package app.photovault.android.data.ml.fake

import app.photovault.android.data.CoreError
import app.photovault.android.data.CoreRepository
import app.photovault.android.data.FileKind
import app.photovault.android.data.PhotoCollection
import app.photovault.android.data.TimelineItem
import app.photovault.android.data.fake.PlaceholderPng
import app.photovault.android.data.ml.FaceRef
import app.photovault.android.data.ml.FileFace
import app.photovault.android.data.ml.Memory
import app.photovault.android.data.ml.MemoryKind
import app.photovault.android.data.ml.MlRepository
import app.photovault.android.data.ml.MlStatus
import app.photovault.android.data.ml.Person
import app.photovault.android.data.ml.SearchKind
import app.photovault.android.data.ml.SearchSuggestion
import kotlinx.coroutines.CoroutineDispatcher
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.withContext
import java.time.Instant
import java.time.LocalDate
import java.time.ZoneOffset
import java.time.format.TextStyle
import java.util.Locale

/**
 * In-memory stand-in for the search / people / memories part of photovault-core, over the demo library of
 * [app.photovault.android.data.fake.FakeCore] (or any [CoreRepository]: it reads the library through
 * `timelinePage` and `collections`, so its file ids are the timeline's).
 *
 * Demo data, deterministic for a given library:
 * - about 45 % of the photos have one to three faces, spread over four named people (Ana, Ben, Chloé and the
 *   hidden Noah) and five unnamed clusters; some clusters look like a named person and are offered as
 *   "review suggestions" for them;
 * - every file has a place (Home, Lisbon, Swiss Alps, Kyoto) and magic search scores files with a hash of
 *   the query, so any description finds a stable handful of photos;
 * - memories: a month ago, a person spotlight, a trip, a CLIP theme and a throwback.
 *
 * Naming, merging, hiding, suggestions and removed faces are kept in memory for the process. Nothing is
 * written anywhere. Not signed in (empty library) = no people, no results, no memories.
 *
 * Face ids use Ente's format (`<fileID>_<xMin>_<yMin>_<xMax>_<yMax>`). Based on Ente (ente.com).
 */
class FakeMlRepository(
    private val core: CoreRepository,
    private val latencyMs: Long = 250L,
    private val ioDispatcher: CoroutineDispatcher = Dispatchers.IO,
) : MlRepository {

    private val lock = Any()

    // ---- state (guarded by lock) ----
    private var enabled = true
    private var pending = INITIAL_PENDING
    private var running = false
    private var library: Library? = null

    /** faceId -> person or cluster id; null = removed from its person (no longer clustered). */
    private val assignment = HashMap<String, String?>()
    private val names = HashMap<String, String>()
    private val hidden = HashSet<String>()
    private val birthDates = HashMap<String, String>()
    private val covers = HashMap<String, String>()
    private val rejected = HashMap<String, MutableSet<String>>()
    private val seenMemories = HashSet<String>()
    private var nextId = 1

    // ---- library snapshot ----

    private class FaceRec(val ref: FaceRef, val base: Int)

    private class Library(
        val items: List<TimelineItem>,
        val indexOf: Map<Long, Int>,
        val collections: List<PhotoCollection>,
        val faces: Map<String, FaceRec>,
        val facesByFile: Map<Long, List<String>>,
        val placeOf: LongArray,
    )

    /** The demo library as core serves it now; rebuilt (and the face state reset) when the library changes. */
    private fun lib(): Library {
        val count = core.timelineCount()
        synchronized(lock) {
            library?.let { if (it.items.size.toLong() == count) return it }
        }
        val items = if (count <= 0) emptyList() else core.timelinePage(0, count.coerceAtMost(Int.MAX_VALUE.toLong()).toInt())
        val built = build(items, core.collections())
        synchronized(lock) {
            val current = library
            if (current != null && current.items.size == built.items.size) return current
            library = built
            assignment.clear()
            built.faces.forEach { (id, f) -> assignment[id] = BASE[f.base].id }
            names.clear()
            hidden.clear()
            birthDates.clear()
            covers.clear()
            rejected.clear()
            BASE.forEach { b ->
                b.name?.let { names[b.id] = it }
                if (b.hidden) hidden += b.id
                b.birthDate?.let { birthDates[b.id] = it }
            }
            return built
        }
    }

    private fun build(items: List<TimelineItem>, collections: List<PhotoCollection>): Library {
        val faces = LinkedHashMap<String, FaceRec>()
        val byFile = HashMap<Long, List<String>>()
        val places = LongArray(items.size)
        val indexOf = HashMap<Long, Int>(items.size * 2)
        for ((i, item) in items.withIndex()) {
            indexOf[item.id] = i
            val rnd = PlaceholderPng.Lcg(item.id * 31 + 7)
            val placeRoll = rnd.nextFloat()
            places[i] = when {
                placeRoll < 0.55f -> 0L
                placeRoll < 0.72f -> 1L
                placeRoll < 0.86f -> 2L
                else -> 3L
            }
            if (item.kind == FileKind.Video || item.kind == FileKind.Unknown) continue
            if (rnd.nextFloat() >= FACE_SHARE) continue
            val n = 1 + (rnd.nextFloat() * 3).toInt().coerceAtMost(2)
            val ids = ArrayList<String>(n)
            val used = HashSet<Int>()
            for (k in 0 until n) {
                val base = pickBase(rnd.nextFloat())
                if (!used.add(base)) continue
                val w = 0.12f + rnd.nextFloat() * 0.18f
                val h = w * item.width / item.height.coerceAtLeast(1)
                val x = (k + 0.5f) / n - w / 2
                val y = 0.15f + rnd.nextFloat() * 0.3f
                val box = FaceRef(
                    faceId = faceId(item.id, x, y, x + w, y + h),
                    fileId = item.id,
                    x = x, y = y, width = w, height = h.coerceAtMost(0.6f),
                    score = 0.75f + rnd.nextFloat() * 0.24f,
                    blur = 40f + rnd.nextFloat() * 200f,
                )
                faces[box.faceId] = FaceRec(box, base)
                ids += box.faceId
            }
            byFile[item.id] = ids
        }
        return Library(items, indexOf, collections, faces, byFile, places)
    }

    // ---- ML control ----

    override fun mlStatus(): MlStatus {
        val total = lib().items.count { it.kind != FileKind.Unknown }.toLong()
        return synchronized(lock) { statusLocked(total) }
    }

    private fun statusLocked(total: Long): MlStatus {
        val left = if (enabled) minOf(pending.toLong(), total) else total
        return MlStatus(
            enabled = enabled,
            inferenceAvailable = true,
            modelsReady = enabled,
            modelBytesDone = if (enabled) MODEL_BYTES else 0L,
            modelBytesTotal = MODEL_BYTES,
            filesTotal = total,
            filesIndexed = total - left,
            filesPending = left,
            running = running,
            lastError = null,
        )
    }

    override suspend fun setMlEnabled(enabled: Boolean): MlStatus {
        delay(latencyMs)
        synchronized(lock) { this.enabled = enabled }
        return mlStatus()
    }

    override suspend fun syncMl(onStatus: ((MlStatus) -> Unit)?): MlStatus {
        delay(latencyMs)
        return mlStatus().also { onStatus?.invoke(it) }
    }

    override suspend fun runMlIndexing(maxFiles: Int, onStatus: ((MlStatus) -> Unit)?): MlStatus {
        if (!synchronized(lock) { enabled }) return mlStatus()
        var budget = maxFiles.coerceAtLeast(0)
        synchronized(lock) { running = true }
        try {
            while (budget > 0 && synchronized(lock) { pending } > 0) {
                delay(latencyMs / 5)
                val step = minOf(budget, 5)
                synchronized(lock) { pending = (pending - step).coerceAtLeast(0) }
                budget -= step
                onStatus?.invoke(mlStatus())
            }
        } finally {
            synchronized(lock) { running = false }
        }
        return mlStatus()
    }

    // ---- search ----

    override fun searchSuggestions(query: String, limit: Int): List<SearchSuggestion> {
        val q = query.trim()
        if (q.isEmpty() || limit <= 0) return emptyList()
        val lib = lib()
        if (lib.items.isEmpty()) return emptyList()
        val lower = q.lowercase(Locale.ROOT)
        val out = ArrayList<SearchSuggestion>()

        // people: a word of the name starts with the query
        peopleLocked(lib, includeHidden = false, minFiles = 1)
            .filter { p -> p.name?.lowercase(Locale.ROOT)?.split(' ')?.any { it.startsWith(lower) } == true }
            .forEach { out += personSuggestion(it) }
        // albums
        lib.collections.filter { lower in it.name.lowercase(Locale.ROOT) }.forEach {
            out += SearchSuggestion(SearchKind.Album, it.id.toString(), it.name, it.count, it.coverFileId, null)
        }
        // places
        PLACES.forEachIndexed { i, name ->
            if (lower in name.lowercase(Locale.ROOT)) out += placeSuggestion(lib, i)
        }
        // dates: a year, or a month name
        q.toIntOrNull()?.takeIf { q.length == 4 }?.let { year ->
            val files = filesFor(lib, SearchKind.Date, "y:$year")
            if (files.isNotEmpty()) out += SearchSuggestion(SearchKind.Date, "y:$year", year.toString(), files.size.toLong(), files.first().id, null)
        }
        if (q.length >= 3) {
            for (m in 1..12) {
                val name = java.time.Month.of(m).getDisplayName(TextStyle.FULL, Locale.ENGLISH)
                if (!name.lowercase(Locale.ROOT).startsWith(lower)) continue
                val key = "m:%02d".format(m)
                val files = filesFor(lib, SearchKind.Date, key)
                if (files.isNotEmpty()) out += SearchSuggestion(SearchKind.Date, key, name, files.size.toLong(), files.first().id, null)
            }
        }
        // file types
        FILE_TYPES.forEach { (key, label) ->
            if (label.lowercase(Locale.ROOT).startsWith(lower) || key.startsWith(lower)) {
                val files = filesFor(lib, SearchKind.FileType, key)
                if (files.isNotEmpty()) out += SearchSuggestion(SearchKind.FileType, key, label, files.size.toLong(), files.first().id, null)
            }
        }
        // file names
        val named = lib.items.filter { lower in it.title.lowercase(Locale.ROOT) }
        if (named.isNotEmpty()) {
            val label = if (named.size == 1) named.first().title else q
            out += SearchSuggestion(SearchKind.FileName, q, label, named.size.toLong(), named.first().id, null)
        }
        // magic: any description of 3+ characters
        if (q.length >= 3 && synchronized(lock) { enabled }) {
            out += SearchSuggestion(SearchKind.Magic, q, q, 0, null, null)
        }
        return out.take(limit)
    }

    override fun searchDiscovery(): List<SearchSuggestion> {
        val lib = lib()
        if (lib.items.isEmpty()) return emptyList()
        val out = ArrayList<SearchSuggestion>()
        peopleLocked(lib, includeHidden = false, minFiles = 3).take(12).forEach { out += personSuggestion(it) }
        PLACES.indices.forEach { out += placeSuggestion(lib, it) }
        FILE_TYPES.forEach { (key, label) ->
            if (key == "image") return@forEach
            val files = filesFor(lib, SearchKind.FileType, key)
            if (files.isNotEmpty()) out += SearchSuggestion(SearchKind.FileType, key, label, files.size.toLong(), files.first().id, null)
        }
        lib.collections.sortedByDescending { it.updatedAtUs }.take(6).forEach {
            out += SearchSuggestion(SearchKind.Album, it.id.toString(), it.name, it.count, it.coverFileId, null)
        }
        if (synchronized(lock) { enabled }) {
            MAGIC_PROMPTS.forEach { out += SearchSuggestion(SearchKind.Magic, it, it, 0, null, null) }
        }
        return out
    }

    override suspend fun searchResults(kind: SearchKind, key: String): List<TimelineItem> = withContext(ioDispatcher) {
        delay(latencyMs / 3)
        filesFor(lib(), kind, key)
    }

    override suspend fun magicSearch(query: String, limit: Int): List<TimelineItem> = withContext(ioDispatcher) {
        delay(latencyMs)
        magic(lib(), query, limit)
    }

    private fun filesFor(lib: Library, kind: SearchKind, key: String): List<TimelineItem> = when (kind) {
        SearchKind.Magic -> magic(lib, key, 200)
        SearchKind.Person -> personFilesOf(lib, key)
        SearchKind.Album -> key.toLongOrNull()?.let { id -> lib.items.filter { it.collectionId == id } }.orEmpty()
        SearchKind.Date -> {
            val (type, value) = key.split(':', limit = 2).let { it[0] to it.getOrElse(1) { "" } }
            lib.items.filter { item ->
                val d = dateOf(item.takenAtUs)
                when (type) {
                    "y" -> d.year.toString() == value
                    "ym" -> "%04d-%02d".format(d.year, d.monthValue) == value
                    "ymd" -> d.toString() == value
                    "md" -> "%02d-%02d".format(d.monthValue, d.dayOfMonth) == value
                    "m" -> "%02d".format(d.monthValue) == value
                    else -> false
                }
            }
        }
        SearchKind.Place -> {
            val place = PLACES.indexOfFirst { "city:$it" == key }
            if (place < 0) emptyList() else lib.items.filterIndexed { i, _ -> lib.placeOf[i] == place.toLong() }
        }
        SearchKind.FileName -> key.lowercase(Locale.ROOT).let { k -> lib.items.filter { k in it.title.lowercase(Locale.ROOT) } }
        SearchKind.Caption -> emptyList()
        SearchKind.FileType -> lib.items.filter {
            when (key) {
                "image" -> it.kind == FileKind.Image
                "video" -> it.kind == FileKind.Video
                "live" -> it.kind == FileKind.LivePhoto
                "screenshot" -> lib.collections.firstOrNull { c -> c.name == "Screenshots" }?.id == it.collectionId
                else -> false
            }
        }
    }

    /** A stable "CLIP score" per (query, file): the best few percent match, best first. */
    private fun magic(lib: Library, query: String, limit: Int): List<TimelineItem> {
        val q = query.trim().lowercase(Locale.ROOT)
        if (q.isEmpty() || limit <= 0 || !synchronized(lock) { enabled }) return emptyList()
        val seed = q.hashCode().toLong()
        return lib.items.asSequence()
            .filter { it.kind != FileKind.Unknown }
            .map { it to PlaceholderPng.Lcg(seed * 1_000_003L + it.id).nextFloat() }
            .filter { it.second > 1f - MAGIC_SHARE }
            .sortedByDescending { it.second }
            .take(limit)
            .map { it.first }
            .toList()
    }

    private fun personSuggestion(p: Person) = SearchSuggestion(
        kind = SearchKind.Person,
        key = p.id,
        label = p.name ?: "",
        count = p.fileCount,
        coverFileId = p.cover?.fileId,
        coverFaceId = p.cover?.faceId,
    )

    private fun placeSuggestion(lib: Library, place: Int): SearchSuggestion {
        val files = filesFor(lib, SearchKind.Place, "city:${PLACES[place]}")
        return SearchSuggestion(SearchKind.Place, "city:${PLACES[place]}", PLACES[place], files.size.toLong(), files.firstOrNull()?.id, null)
    }

    // ---- people ----

    override fun people(includeHidden: Boolean, minFiles: Int): List<Person> = peopleLocked(lib(), includeHidden, minFiles)

    private fun peopleLocked(lib: Library, includeHidden: Boolean, minFiles: Int): List<Person> {
        synchronized(lock) {
            if (!enabled) return emptyList()
            val ids = assignment.values.filterNotNull().toSet()
            val all = ids.mapNotNull { personLocked(lib, it) }.filter { includeHidden || !it.isHidden }
            val namedPeople = all.filter { it.name != null }.sortedWith(compareByDescending<Person> { it.fileCount }.thenBy { it.id })
            val clusters = all.filter { it.name == null && it.fileCount >= minFiles }
                .sortedWith(compareByDescending<Person> { it.fileCount }.thenBy { it.id })
            return namedPeople + clusters
        }
    }

    override fun person(personId: String): Person? {
        val lib = lib()
        return synchronized(lock) { if (enabled) personLocked(lib, personId) else null }
    }

    private fun personLocked(lib: Library, id: String): Person? {
        val faces = assignment.entries.filter { it.value == id }.mapNotNull { lib.faces[it.key]?.ref }
        if (faces.isEmpty()) return null
        val cover = covers[id]?.let { c -> faces.firstOrNull { it.faceId == c } }
            ?: faces.maxWith(compareBy<FaceRef> { it.score }.thenBy { it.faceId })
        return Person(
            id = id,
            name = names[id],
            isHidden = id in hidden,
            fileCount = faces.map { it.fileId }.distinct().size.toLong(),
            cover = cover,
            birthDate = birthDates[id],
        )
    }

    override fun personFiles(personId: String): List<TimelineItem> = personFilesOf(lib(), personId)

    private fun personFilesOf(lib: Library, personId: String): List<TimelineItem> {
        val files = synchronized(lock) {
            if (!enabled) return emptyList()
            assignment.entries.filter { it.value == personId }.mapNotNull { lib.faces[it.key]?.ref?.fileId }.toSet()
        }
        // newest first = timeline order
        return files.mapNotNull { lib.indexOf[it] }.sorted().map { lib.items[it] }
    }

    override fun fileFaces(fileId: Long): List<FileFace> {
        val lib = lib()
        val ids = lib.facesByFile[fileId] ?: return emptyList()
        return synchronized(lock) {
            if (!enabled) return emptyList()
            ids.mapNotNull { id ->
                val f = lib.faces[id] ?: return@mapNotNull null
                val person = assignment[id]
                FileFace(f.ref, person, person?.let { names[it] })
            }
        }
    }

    override fun personSuggestions(personId: String): List<Person> {
        val lib = lib()
        synchronized(lock) {
            if (!enabled || names[personId] == null) return emptyList()
            // a cluster is suggested for the person it was seeded to look like, until answered
            val base = BASE.indexOfFirst { it.id == personId }.takeIf { it >= 0 }
            val no = rejected[personId].orEmpty()
            return BASE.filter { it.name == null && it.looksLike != null && it.looksLike == base && it.id !in no }
                .mapNotNull { personLocked(lib, it.id) }
                .filter { it.name == null }
        }
    }

    override suspend fun faceThumbnail(faceId: String): ByteArray = withContext(ioDispatcher) {
        val face = lib().faces[faceId] ?: throw CoreError.Storage("No face $faceId")
        // one look per person, so a person's faces are recognisable in the demo
        PlaceholderPng.render(7_919L * (face.base + 1), FACE_PX, FACE_PX)
    }

    override suspend fun namePerson(personId: String, name: String): Person {
        val clean = name.trim()
        if (clean.isEmpty()) throw CoreError.Storage("A name can't be empty")
        delay(latencyMs)
        val lib = lib()
        synchronized(lock) {
            personLocked(lib, personId) ?: throw CoreError.Storage("No person $personId")
            val id = if (personId.startsWith(CLUSTER_PREFIX)) {
                val fresh = "pv-${nextId++}"
                reassignLocked(from = personId, to = fresh)
                fresh
            } else {
                personId
            }
            names[id] = clean
            return personLocked(lib, id)!!
        }
    }

    override suspend fun mergePeople(intoId: String, fromId: String): Person {
        if (intoId == fromId) throw CoreError.Storage("Can't merge a person with itself")
        delay(latencyMs)
        val lib = lib()
        synchronized(lock) {
            personLocked(lib, intoId) ?: throw CoreError.Storage("No person $intoId")
            personLocked(lib, fromId) ?: throw CoreError.Storage("No person $fromId")
            reassignLocked(from = fromId, to = intoId)
            names.remove(fromId)
            hidden.remove(fromId)
            birthDates.remove(fromId)
            covers.remove(fromId)
            return personLocked(lib, intoId)!!
        }
    }

    override suspend fun answerSuggestion(personId: String, clusterId: String, accept: Boolean) {
        delay(latencyMs / 2)
        val lib = lib()
        synchronized(lock) {
            personLocked(lib, personId) ?: throw CoreError.Storage("No person $personId")
            if (accept) {
                personLocked(lib, clusterId) ?: throw CoreError.Storage("No cluster $clusterId")
                reassignLocked(from = clusterId, to = personId)
            } else {
                rejected.getOrPut(personId) { HashSet() } += clusterId
            }
        }
    }

    override suspend fun setPersonHidden(personId: String, hidden: Boolean) {
        delay(latencyMs / 2)
        val lib = lib()
        synchronized(lock) {
            personLocked(lib, personId) ?: throw CoreError.Storage("No person $personId")
            if (hidden) this.hidden += personId else this.hidden -= personId
        }
    }

    override suspend fun setPersonCover(personId: String, faceId: String) {
        delay(latencyMs / 2)
        synchronized(lock) {
            if (assignment[faceId] != personId) throw CoreError.Storage("That face is not $personId")
            covers[personId] = faceId
        }
    }

    override suspend fun setPersonBirthDate(personId: String, birthDate: String?) {
        delay(latencyMs / 2)
        if (birthDate != null && runCatching { LocalDate.parse(birthDate) }.isFailure) {
            throw CoreError.Storage("Not a date: $birthDate")
        }
        val lib = lib()
        synchronized(lock) {
            personLocked(lib, personId) ?: throw CoreError.Storage("No person $personId")
            if (birthDate == null) birthDates.remove(personId) else birthDates[personId] = birthDate
        }
    }

    override suspend fun removeFacesFromPerson(personId: String, faceIds: List<String>) {
        delay(latencyMs / 2)
        synchronized(lock) {
            faceIds.forEach { if (assignment[it] == personId) assignment[it] = null }
            covers[personId]?.let { if (it in faceIds) covers.remove(personId) }
        }
    }

    override suspend fun deletePerson(personId: String) {
        delay(latencyMs / 2)
        val lib = lib()
        synchronized(lock) {
            personLocked(lib, personId) ?: throw CoreError.Storage("No person $personId")
            if (!personId.startsWith(CLUSTER_PREFIX)) {
                // like core: the person's faces become an unnamed cluster again
                reassignLocked(from = personId, to = "$CLUSTER_PREFIX${nextId++}")
            }
            names.remove(personId)
            hidden.remove(personId)
            birthDates.remove(personId)
            covers.remove(personId)
        }
    }

    private fun reassignLocked(from: String, to: String) {
        for (e in assignment.entries) if (e.value == from) e.setValue(to)
    }

    // ---- memories ----

    override fun memories(nowUs: Long, utcOffsetSecs: Int): List<Memory> {
        val lib = lib()
        if (lib.items.isEmpty()) return emptyList()
        val offset = ZoneOffset.ofTotalSeconds(utcOffsetSecs.coerceIn(-18 * 3600, 18 * 3600))
        val today = dateOf(nowUs, offset)
        val out = ArrayList<Memory>()
        fun add(id: String, kind: MemoryKind, title: String, subtitle: String?, files: List<TimelineItem>, personId: String? = null) {
            if (files.isEmpty()) return
            // story order: oldest first
            val story = files.sortedBy { it.takenAtUs }
            out += Memory(id, kind, title, subtitle, story, story.first().id, synchronized(lock) { id in seenMemories }, personId)
        }

        // this day in earlier years
        add(
            "on-this-day:${today.monthValue}-${today.dayOfMonth}",
            MemoryKind.OnThisDay,
            "On this day",
            null,
            lib.items.filter { dateOf(it.takenAtUs, offset).let { d -> d.year < today.year && d.monthValue == today.monthValue && d.dayOfMonth == today.dayOfMonth } }.take(MEMORY_MAX),
        )
        // about a month ago
        val monthAgo = lib.items.filter {
            val d = dateOf(it.takenAtUs, offset)
            !d.isBefore(today.minusDays(35)) && !d.isAfter(today.minusDays(25))
        }
        add("time:${today.minusMonths(1)}", MemoryKind.Time, "A month ago", null, monthAgo.take(MEMORY_MAX))
        // the person with most photos
        peopleLocked(lib, includeHidden = false, minFiles = 3).firstOrNull { it.name != null }?.let { p ->
            add("people:${p.id}", MemoryKind.People, p.name!!, "Spotlight", personFilesOf(lib, p.id).take(MEMORY_MAX), p.id)
        }
        // a trip
        val trip = filesFor(lib, SearchKind.Place, "city:Lisbon")
        if (trip.isNotEmpty()) {
            val year = dateOf(trip.first().takenAtUs, offset).year
            add("trip:lisbon-$year", MemoryKind.Trip, "Trip to Lisbon", year.toString(), trip.take(MEMORY_MAX))
        }
        // a CLIP theme
        add("clip:sunsets", MemoryKind.Clip, "Sunsets", null, magic(lib, "sunset", MEMORY_MAX))
        // a throwback from the older half of the library
        val older = lib.items.drop(lib.items.size / 2)
        val rnd = PlaceholderPng.Lcg(today.toEpochDay())
        add("filler:${today}", MemoryKind.Filler, "Throwback", null, older.filter { rnd.nextFloat() < 0.04f }.take(12))

        return out.sortedBy { it.seen }
    }

    override fun markMemorySeen(memoryId: String) {
        synchronized(lock) { seenMemories += memoryId }
    }

    // ---- helpers ----

    private fun dateOf(us: Long, offset: ZoneOffset = ZoneOffset.UTC): LocalDate =
        Instant.ofEpochSecond(Math.floorDiv(us, 1_000_000L)).atOffset(offset).toLocalDate()

    private class Base(
        val id: String,
        val name: String?,
        val weight: Float,
        val hidden: Boolean = false,
        val birthDate: String? = null,
        /** For an unnamed cluster: the [BASE] index of the named person it is suggested for. */
        val looksLike: Int? = null,
    )

    companion object {
        const val CLUSTER_PREFIX = "cluster:"
        private const val INITIAL_PENDING = 40
        private const val MODEL_BYTES = 248_000_000L
        private const val FACE_SHARE = 0.45f
        private const val MAGIC_SHARE = 0.06f
        private const val MEMORY_MAX = 30
        private const val FACE_PX = 160

        val PLACES = listOf("Home", "Lisbon", "Swiss Alps", "Kyoto")
        val MAGIC_PROMPTS = listOf("Sunset", "Food", "Mountains", "Beach", "Dogs", "Snow")
        private val FILE_TYPES = listOf("image" to "Photos", "video" to "Videos", "live" to "Live photos", "screenshot" to "Screenshots")

        private val BASE = listOf(
            Base("pv-ana", "Ana", 0.24f, birthDate = "1992-04-17"),
            Base("pv-ben", "Ben", 0.18f),
            Base("pv-chloe", "Chloé", 0.12f),
            Base("pv-noah", "Noah", 0.06f, hidden = true),
            Base("${CLUSTER_PREFIX}c1", null, 0.12f, looksLike = 0),
            Base("${CLUSTER_PREFIX}c2", null, 0.10f, looksLike = 0),
            Base("${CLUSTER_PREFIX}c3", null, 0.08f, looksLike = 1),
            Base("${CLUSTER_PREFIX}c4", null, 0.06f, looksLike = 2),
            Base("${CLUSTER_PREFIX}c5", null, 0.04f),
        )

        private fun pickBase(roll: Float): Int {
            var acc = 0f
            for ((i, b) in BASE.withIndex()) {
                acc += b.weight
                if (roll < acc) return i
            }
            return BASE.lastIndex
        }

        /** Ente's face id: `<fileID>_<xMin>_<yMin>_<xMax>_<yMax>`, each coordinate as 5 decimals without "0.". */
        internal fun faceId(fileId: Long, x0: Float, y0: Float, x1: Float, y1: Float): String {
            fun part(v: Float) = "%.5f".format(Locale.ROOT, v.coerceIn(0f, 0.99999f)).substringAfter('.')
            return "${fileId}_${part(x0)}_${part(y0)}_${part(x1)}_${part(y1)}"
        }
    }
}
