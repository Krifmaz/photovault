package app.photovault.android.ui.people

import app.photovault.android.MainDispatcherRule
import app.photovault.android.data.fake.FakeCore
import app.photovault.android.data.ml.Person
import app.photovault.android.data.ml.fake.FakeMlRepository
import app.photovault.android.ui.search.PhotoSource
import app.photovault.android.ui.search.ResultLists
import app.photovault.android.ui.timeline.GridEntry
import app.photovault.android.ui.timeline.TimelineLabels
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.test.advanceUntilIdle
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import java.time.LocalDate
import java.time.ZoneOffset
import java.util.Locale

@OptIn(ExperimentalCoroutinesApi::class)
class PeopleViewModelsTest {
    @get:Rule val main = MainDispatcherRule()

    private val now = 1_793_664_000_000_000L // 2026-11-03T00:00:00Z
    private val core = FakeCore(nowUs = now, itemCount = 640, latencyMs = 0, ioDispatcher = main.dispatcher)
    private val ml = FakeMlRepository(core, latencyMs = 10, ioDispatcher = main.dispatcher)
    private val labels = TimelineLabels("Today", "Yesterday", ZoneOffset.UTC, Locale.US) { LocalDate.of(2026, 11, 3) }

    private suspend fun signIn() {
        core.addServer("photos.example.com")
        core.login("me@example.com", "secret")
    }

    private fun named(name: String): Person = ml.people(includeHidden = true, minFiles = 1).first { it.name == name }

    private fun cluster(): Person = ml.people(includeHidden = false, minFiles = 1).first { it.name == null }

    private fun person(id: String) = PersonViewModel(ml, id, labels, io = main.dispatcher, compute = main.dispatcher)

    @Test
    fun peopleListAndHiddenToggle() = runTest(main.dispatcher) {
        signIn()
        val vm = PeopleViewModel(ml, io = main.dispatcher)
        advanceUntilIdle()
        val s = vm.state.value
        assertTrue(s.loaded)
        assertTrue(s.people.isNotEmpty())
        assertTrue(s.people.all { it.name != null || it.fileCount >= PeopleViewModel.MIN_FILES })
        assertFalse(s.people.any { it.isHidden })
        vm.setShowHidden(true)
        advanceUntilIdle()
        assertTrue(vm.state.value.people.any { it.isHidden && it.name == "Noah" })
    }

    @Test
    fun personShowsItsPhotosUnderMonthHeaders() = runTest(main.dispatcher) {
        signIn()
        val ana = named("Ana")
        val vm = person(ana.id)
        advanceUntilIdle()
        val s = vm.state.value
        assertEquals("Ana", s.person?.name)
        val cells = s.grid.entries.filterIsInstance<GridEntry.Cell>()
        assertEquals(ana.fileCount.toInt(), cells.size)
        assertEquals(2, s.suggestionCount)
        // the viewer opened from here pages through the same photos
        vm.prepareViewer()
        assertEquals(cells.map { it.item.id }, ResultLists.load(ml, PhotoSource.person(ana.id)).map { it.id })
    }

    @Test
    fun removeWrongPhotos() = runTest(main.dispatcher) {
        signIn()
        val ben = named("Ben")
        val vm = person(ben.id)
        advanceUntilIdle()
        val wrong = ml.personFiles(ben.id).take(2).map { it.id }
        vm.toggle(wrong[0]) // ignored outside selection mode
        assertTrue(vm.state.value.selected.isEmpty())
        vm.startSelecting()
        wrong.forEach(vm::toggle)
        vm.toggle(wrong[1])
        vm.toggle(wrong[1])
        assertEquals(wrong.toSet(), vm.state.value.selected)
        vm.removeSelected()
        advanceUntilIdle()
        val s = vm.state.value
        assertFalse(s.selecting)
        assertEquals(ben.fileCount - 2, s.person?.fileCount)
        assertTrue(ml.personFiles(ben.id).none { it.id in wrong })
    }

    @Test
    fun hideAndRemoveName() = runTest(main.dispatcher) {
        signIn()
        val chloe = named("Chloé")
        val vm = person(chloe.id)
        advanceUntilIdle()
        vm.setHidden(true)
        advanceUntilIdle()
        assertEquals(true, vm.state.value.person?.isHidden)
        vm.setHidden(false)
        advanceUntilIdle()
        vm.removeName()
        advanceUntilIdle()
        assertEquals(PersonEvent.Gone, vm.eventFlow.first())
        assertNull(ml.person(chloe.id))
    }

    @Test
    fun useAsCover() = runTest(main.dispatcher) {
        signIn()
        val ana = named("Ana")
        val vm = person(ana.id)
        advanceUntilIdle()
        val other = ml.personFiles(ana.id).first { it.id != ana.cover?.fileId }
        vm.useAsCover(other.id)
        advanceUntilIdle()
        assertEquals(other.id, vm.state.value.person?.cover?.fileId)
    }

    @Test
    fun namingAClusterMovesThePersonScreenToTheNewId() = runTest(main.dispatcher) {
        signIn()
        val c = cluster()
        val name = NamePersonViewModel(ml, c.id, io = main.dispatcher)
        advanceUntilIdle()
        assertEquals("", name.state.value.name)
        name.onName("Dora")
        assertTrue(name.state.value.matches.isEmpty())
        name.save()
        advanceUntilIdle()
        val done = name.doneFlow.first()
        assertEquals("Dora", ml.person(done.personId)?.name)

        val vm = person(c.id)
        advanceUntilIdle()
        vm.onPersonChanged(done.personId)
        advanceUntilIdle()
        assertEquals(done.personId, vm.state.value.personId)
        assertEquals("Dora", vm.state.value.person?.name)
    }

    @Test
    fun typingAnExistingNameOffersAndDoesAMerge() = runTest(main.dispatcher) {
        signIn()
        val c = cluster()
        val ana = named("Ana")
        val vm = NamePersonViewModel(ml, c.id, io = main.dispatcher)
        advanceUntilIdle()
        vm.onName("an")
        assertEquals(listOf("Ana"), vm.state.value.matches.map { it.name })
        vm.onName("ana") // exact (any case) = merge, not a second "Ana"
        vm.save()
        advanceUntilIdle()
        assertEquals(ana.id, vm.doneFlow.first().personId)
        assertNull(ml.person(c.id))
        assertEquals(1, ml.people(includeHidden = true, minFiles = 1).count { it.name == "Ana" })
    }

    @Test
    fun mergeKeepsTheNamedPerson() = runTest(main.dispatcher) {
        signIn()
        val c = cluster()
        val ben = named("Ben")
        assertEquals(ben.id to c.id, MergePeopleViewModel.direction(ben, c))
        assertEquals(ben.id to c.id, MergePeopleViewModel.direction(c, ben))

        val vm = MergePeopleViewModel(ml, c.id, io = main.dispatcher)
        advanceUntilIdle()
        assertFalse(vm.state.value.candidates.any { it.id == c.id })
        vm.merge(ben)
        advanceUntilIdle()
        assertEquals(ben.id, vm.doneFlow.first().personId)
        assertNull(ml.person(c.id))
        assertTrue(ml.person(ben.id)!!.fileCount >= ben.fileCount)
    }

    @Test
    fun reviewSuggestions() = runTest(main.dispatcher) {
        signIn()
        val ana = named("Ana")
        val vm = SuggestionsViewModel(ml, ana.id, io = main.dispatcher)
        advanceUntilIdle()
        val cards = vm.state.value.cards
        assertEquals(2, cards.size)
        assertTrue(cards.all { it.sampleFileIds.isNotEmpty() && it.sampleFileIds.size <= SuggestionsViewModel.SAMPLES })
        vm.answer(cards[0], accept = true)
        vm.answer(cards[1], accept = false)
        advanceUntilIdle()
        val s = vm.state.value
        assertTrue(s.cards.isEmpty())
        assertEquals(1, s.accepted)
        assertTrue(s.answering.isEmpty())
        assertNull(ml.person(cards[0].cluster.id))
        assertNotNull(ml.person(cards[1].cluster.id))
        assertTrue(ml.personSuggestions(ana.id).isEmpty())
    }
}
