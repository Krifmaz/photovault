package app.photovault.android.ui.search

import app.photovault.android.MainDispatcherRule
import app.photovault.android.data.TimelineStore
import app.photovault.android.data.fake.FakeCore
import app.photovault.android.data.ml.SearchKind
import app.photovault.android.data.ml.fake.FakeMlRepository
import app.photovault.android.ui.timeline.GridEntry
import app.photovault.android.ui.timeline.TimelineLabels
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.advanceTimeBy
import kotlinx.coroutines.test.advanceUntilIdle
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import java.time.LocalDate
import java.time.ZoneOffset
import java.util.Locale

@OptIn(ExperimentalCoroutinesApi::class)
class SearchViewModelTest {
    @get:Rule val main = MainDispatcherRule()

    private val now = 1_793_664_000_000_000L // 2026-11-03T00:00:00Z
    private val core = FakeCore(nowUs = now, itemCount = 640, latencyMs = 0, ioDispatcher = main.dispatcher)
    private val ml = FakeMlRepository(core, latencyMs = 20, ioDispatcher = main.dispatcher)
    private val labels = TimelineLabels("Today", "Yesterday", ZoneOffset.UTC, Locale.US) { LocalDate.of(2026, 11, 3) }

    private suspend fun signIn() {
        core.addServer("photos.example.com")
        core.login("me@example.com", "secret")
    }

    @Test
    fun discoveryIsGroupedByKind() = runTest(main.dispatcher) {
        signIn()
        val vm = SearchViewModel(ml, io = main.dispatcher)
        advanceUntilIdle()
        val s = vm.state.value
        assertTrue(s.discoveryLoaded)
        assertTrue(s.mlEnabled)
        val d = s.discovery
        assertTrue(d.people.isNotEmpty() && d.people.all { it.kind == SearchKind.Person && it.coverFaceId != null })
        assertTrue(d.places.isNotEmpty() && d.places.all { it.kind == SearchKind.Place })
        assertTrue(d.fileTypes.isNotEmpty())
        assertTrue(d.albums.isNotEmpty())
        assertTrue(d.magic.isNotEmpty())
    }

    @Test
    fun typingIsDebouncedAndOnlyTheLastQueryIsAnswered() = runTest(main.dispatcher) {
        signIn()
        val vm = SearchViewModel(ml, io = main.dispatcher, debounceMs = 250)
        advanceUntilIdle()
        vm.onQuery("h")
        vm.onQuery("ho")
        vm.onQuery("holi")
        assertTrue(vm.state.value.searching)
        advanceTimeBy(100)
        assertTrue("still within the debounce", vm.state.value.groups.isEmpty())
        advanceUntilIdle()
        val s = vm.state.value
        assertFalse(s.searching)
        assertEquals("holi", s.answered)
        assertEquals(SearchKind.Album, s.groups.first().kind)
        assertEquals("Holidays", s.groups.first().items.single().label)
        // groups follow the screen's order
        val order = s.groups.map { SearchViewModel.ORDER.indexOf(it.kind) }
        assertEquals(order.sorted(), order)
    }

    @Test
    fun noResultsAndClear() = runTest(main.dispatcher) {
        signIn()
        val vm = SearchViewModel(ml, io = main.dispatcher)
        advanceUntilIdle()
        vm.onQuery("zq")
        advanceUntilIdle()
        assertTrue(vm.state.value.noResults)
        vm.clear()
        advanceUntilIdle()
        assertEquals("", vm.state.value.query)
        assertTrue(vm.state.value.groups.isEmpty())
        assertFalse(vm.state.value.noResults)
        assertNull(vm.submitTarget())
    }

    @Test
    fun submitOpensTheBestSuggestionOrAMagicSearch() = runTest(main.dispatcher) {
        signIn()
        val vm = SearchViewModel(ml, io = main.dispatcher)
        advanceUntilIdle()
        vm.onQuery("Ana")
        // before the suggestions arrive: magic search for the text
        assertEquals(SearchKind.Magic, vm.submitTarget()?.kind)
        advanceUntilIdle()
        val target = vm.submitTarget()!!
        assertEquals(SearchKind.Person, target.kind)
        assertEquals("Ana", target.label)
    }

    @Test
    fun enablingMlBringsPeopleBack() = runTest(main.dispatcher) {
        signIn()
        ml.setMlEnabled(false)
        val vm = SearchViewModel(ml, io = main.dispatcher)
        advanceUntilIdle()
        assertFalse(vm.state.value.mlEnabled)
        assertTrue(vm.state.value.discovery.people.isEmpty())
        vm.enableMl()
        advanceUntilIdle()
        assertTrue(vm.state.value.mlEnabled)
        assertFalse(vm.state.value.mlBusy)
        assertTrue(vm.state.value.discovery.people.isNotEmpty())
    }

    @Test
    fun resultsAreGroupedByMonthExceptMagic() = runTest(main.dispatcher) {
        signIn()
        val album = SearchResultsViewModel(ml, SearchKind.Album, "2", labels, compute = main.dispatcher)
        advanceUntilIdle()
        val a = album.state.value
        assertTrue(a.loaded)
        assertEquals(ml.searchResults(SearchKind.Album, "2").size, a.count)
        assertTrue(a.grid.entries.first() is GridEntry.Header)
        assertTrue(a.grid.entries.filterIsInstance<GridEntry.Header>().all { it.key.startsWith("m") })

        val magic = SearchResultsViewModel(ml, SearchKind.Magic, "beach", labels, compute = main.dispatcher)
        advanceUntilIdle()
        val m = magic.state.value
        assertTrue(m.count > 0)
        assertTrue(m.grid.entries.all { it is GridEntry.Cell })
        // the viewer pages through the same list, in the same order
        magic.prepareViewer()
        val shown = m.grid.entries.map { (it as GridEntry.Cell).item.id }
        assertEquals(shown, ResultLists.load(ml, magic.source).map { it.id })
    }

    @Test
    fun theViewerStoreServesAResultList() = runTest(main.dispatcher) {
        signIn()
        val people = ml.people(includeHidden = false, minFiles = 1)
        val ana = people.first { it.name == "Ana" }
        ResultLists.clear()
        val store = TimelineStore(ListCore(core) { ResultLists.load(ml, PhotoSource.person(ana.id)) }, io = main.dispatcher)
        store.refresh()
        assertEquals(ana.fileCount, store.total.value)
        assertEquals(ml.personFiles(ana.id).take(TimelineStore.CHUNK), store.items.value)
        // a page beyond the end is empty
        assertTrue(ListCore(core) { ml.personFiles(ana.id) }.timelinePage(10_000, 10).isEmpty())
    }
}
