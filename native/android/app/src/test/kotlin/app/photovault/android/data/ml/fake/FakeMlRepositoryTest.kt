package app.photovault.android.data.ml.fake

import app.photovault.android.data.CoreError
import app.photovault.android.data.FileKind
import app.photovault.android.data.fake.FakeCore
import app.photovault.android.data.ml.MemoryKind
import app.photovault.android.data.ml.SearchKind
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Test

class FakeMlRepositoryTest {
    private val now = 1_793_664_000_000_000L // 2026-11-03T00:00:00Z
    private val core = FakeCore(nowUs = now, itemCount = 640, latencyMs = 0)
    private val ml = FakeMlRepository(core, latencyMs = 0)

    private suspend fun signIn() {
        core.addServer("photos.example.com")
        core.login("me@example.com", "secret")
    }

    @Test
    fun nothingBeforeSignIn() = runTest {
        assertTrue(ml.people(includeHidden = true, minFiles = 1).isEmpty())
        assertTrue(ml.searchDiscovery().isEmpty())
        assertTrue(ml.memories(now, 0).isEmpty())
    }

    @Test
    fun peopleAreNamedFirstAndUseTheDemoLibrarysFiles() = runTest {
        signIn()
        val ids = core.timelinePage(0, 640).map { it.id }.toSet()
        val people = ml.people(includeHidden = false, minFiles = 1)
        val names = people.mapNotNull { it.name }
        assertEquals(listOf("Ana", "Ben", "Chloé"), names.sorted().take(3))
        assertFalse("Noah is hidden", "Noah" in names)
        assertTrue(people.indexOfLast { it.name != null } < people.indexOfFirst { it.name == null })
        for (p in people) {
            val files = ml.personFiles(p.id)
            assertEquals(p.fileCount, files.size.toLong())
            assertTrue(files.all { it.id in ids })
            assertEquals(files.sortedByDescending { it.takenAtUs }.map { it.id }, files.map { it.id })
            assertNotNull(p.cover)
            val cover = p.cover!!
            assertTrue(cover.fileId in files.map { it.id })
            assertTrue(cover.faceId.startsWith("${cover.fileId}_"))
            assertTrue(ml.fileFaces(cover.fileId).any { it.face.faceId == cover.faceId && it.personId == p.id })
        }
        assertTrue(ml.people(includeHidden = true, minFiles = 1).any { it.name == "Noah" && it.isHidden })
        val face = people.first().cover!!.faceId
        assertTrue(ml.faceThumbnail(face).isNotEmpty())
    }

    @Test
    fun namingAClusterGivesItAnIdAndMergingMovesItsFaces() = runTest {
        signIn()
        val cluster = ml.people(includeHidden = false, minFiles = 1).first { it.name == null }
        assertTrue(cluster.id.startsWith(FakeMlRepository.CLUSTER_PREFIX))
        val named = ml.namePerson(cluster.id, "  Dora ")
        assertEquals("Dora", named.name)
        assertFalse(named.id.startsWith(FakeMlRepository.CLUSTER_PREFIX))
        assertNull(ml.person(cluster.id))
        assertEquals(cluster.fileCount, named.fileCount)

        val ana = ml.people(includeHidden = false, minFiles = 1).first { it.name == "Ana" }
        val merged = ml.mergePeople(intoId = ana.id, fromId = named.id)
        assertEquals("Ana", merged.name)
        assertNull(ml.person(named.id))
        assertTrue(merged.fileCount >= ana.fileCount)

        try {
            ml.namePerson(ana.id, " ")
            fail("blank name accepted")
        } catch (e: CoreError.Storage) {
            // expected
        }
    }

    @Test
    fun suggestionsCanBeAcceptedOrRejected() = runTest {
        signIn()
        val ana = ml.people(includeHidden = false, minFiles = 1).first { it.name == "Ana" }
        val suggestions = ml.personSuggestions(ana.id)
        assertEquals(2, suggestions.size)
        ml.answerSuggestion(ana.id, suggestions[0].id, accept = true)
        ml.answerSuggestion(ana.id, suggestions[1].id, accept = false)
        assertTrue(ml.personSuggestions(ana.id).isEmpty())
        assertNull(ml.person(suggestions[0].id))
        assertNotNull(ml.person(suggestions[1].id))
        assertTrue(ml.person(ana.id)!!.fileCount > ana.fileCount)
    }

    @Test
    fun removingFacesHidingAndDeleting() = runTest {
        signIn()
        val ben = ml.people(includeHidden = false, minFiles = 1).first { it.name == "Ben" }
        val file = ml.personFiles(ben.id).first()
        val faces = ml.fileFaces(file.id).filter { it.personId == ben.id }.map { it.face.faceId }
        ml.removeFacesFromPerson(ben.id, faces)
        assertFalse(ml.personFiles(ben.id).any { it.id == file.id })
        assertTrue(ml.fileFaces(file.id).filter { it.face.faceId in faces }.all { it.personId == null })

        ml.setPersonHidden(ben.id, true)
        assertFalse(ml.people(includeHidden = false, minFiles = 1).any { it.id == ben.id })
        ml.setPersonHidden(ben.id, false)

        ml.setPersonBirthDate(ben.id, "1990-02-03")
        assertEquals("1990-02-03", ml.person(ben.id)!!.birthDate)

        ml.deletePerson(ben.id)
        assertNull(ml.person(ben.id))
        assertFalse(ml.people(includeHidden = true, minFiles = 1).any { it.name == "Ben" })
    }

    @Test
    fun searchSuggestionsAndResultsAgree() = runTest {
        signIn()
        val people = ml.searchSuggestions("an", 20).filter { it.kind == SearchKind.Person }
        assertEquals(listOf("Ana"), people.map { it.label })
        assertNotNull(people.first().coverFaceId)

        val album = ml.searchSuggestions("holi", 20).single { it.kind == SearchKind.Album }
        val inAlbum = ml.searchResults(SearchKind.Album, album.key)
        assertEquals(album.count, inAlbum.size.toLong())
        assertTrue(inAlbum.all { it.collectionId.toString() == album.key })

        val videos = ml.searchResults(SearchKind.FileType, "video")
        assertTrue(videos.isNotEmpty() && videos.all { it.kind == FileKind.Video })

        val place = ml.searchSuggestions("lisb", 20).single { it.kind == SearchKind.Place }
        assertEquals(place.count, ml.searchResults(SearchKind.Place, place.key).size.toLong())

        val name = ml.searchSuggestions("IMG_0001", 20).single { it.kind == SearchKind.FileName }
        assertEquals("IMG_0001.jpg", name.label)

        val magic = ml.searchSuggestions("sunset", 20).last()
        assertEquals(SearchKind.Magic, magic.kind)
        val first = ml.searchResults(SearchKind.Magic, "sunset")
        assertTrue(first.isNotEmpty())
        assertEquals(first, ml.magicSearch("Sunset", 200)) // stable, case-insensitive
        assertTrue(ml.searchSuggestions("", 20).isEmpty())
    }

    @Test
    fun discoveryHasEveryKindTheScreenShows() = runTest {
        signIn()
        val kinds = ml.searchDiscovery().map { it.kind }.toSet()
        assertTrue(kinds.containsAll(listOf(SearchKind.Person, SearchKind.Place, SearchKind.FileType, SearchKind.Album, SearchKind.Magic)))
    }

    @Test
    fun turningMlOffHidesPeopleAndMagic() = runTest {
        signIn()
        assertTrue(ml.mlStatus().enabled)
        ml.setMlEnabled(false)
        assertFalse(ml.mlStatus().enabled)
        assertTrue(ml.people(includeHidden = true, minFiles = 1).isEmpty())
        assertTrue(ml.magicSearch("dog", 10).isEmpty())
        assertFalse(ml.searchDiscovery().any { it.kind == SearchKind.Magic || it.kind == SearchKind.Person })
        ml.setMlEnabled(true)
        assertTrue(ml.people(includeHidden = false, minFiles = 1).isNotEmpty())
    }

    @Test
    fun indexingReportsProgressUntilNothingIsPending() = runTest {
        signIn()
        val before = ml.mlStatus()
        assertTrue(before.filesPending > 0)
        val seen = ArrayList<Long>()
        val after = ml.runMlIndexing(1000) { seen += it.filesPending }
        assertEquals(0L, after.filesPending)
        assertEquals(after.filesTotal, after.filesIndexed)
        assertFalse(after.running)
        assertTrue(seen.isNotEmpty() && seen.zipWithNext().all { (a, b) -> b <= a })
    }

    @Test
    fun memoriesAreStoriesOfLibraryPhotosAndRememberSeen() = runTest {
        signIn()
        val memories = ml.memories(now, 3600)
        assertTrue(memories.size >= 3)
        assertTrue(memories.map { it.id }.toSet().size == memories.size)
        for (m in memories) {
            assertTrue(m.items.isNotEmpty())
            assertEquals(m.items.first().id, m.coverFileId)
            assertEquals(m.items.sortedBy { it.takenAtUs }, m.items)
        }
        assertTrue(memories.any { it.kind == MemoryKind.People && it.personId != null })
        val first = memories.first()
        ml.markMemorySeen(first.id)
        val again = ml.memories(now, 3600)
        assertTrue(again.single { it.id == first.id }.seen)
        assertEquals(first.id, again.last().id) // seen ones go last
    }
}
