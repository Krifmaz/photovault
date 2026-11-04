package app.photovault.android.ui.nav

import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.remember
import androidx.compose.ui.platform.LocalContext
import androidx.lifecycle.viewmodel.compose.viewModel
import androidx.navigation.NavGraphBuilder
import androidx.navigation.NavHostController
import androidx.navigation.compose.composable
import androidx.navigation.toRoute
import app.photovault.android.AppGraph
import app.photovault.android.data.TimelineStore
import app.photovault.android.data.ml.MlWork
import app.photovault.android.data.ml.SearchKind
import app.photovault.android.data.ml.SearchSuggestion
import app.photovault.android.ui.people.MergePeopleScreen
import app.photovault.android.ui.people.MergePeopleViewModel
import app.photovault.android.ui.people.NamePersonScreen
import app.photovault.android.ui.people.NamePersonViewModel
import app.photovault.android.ui.people.PeopleScreen
import app.photovault.android.ui.people.PeopleViewModel
import app.photovault.android.ui.people.PersonScreen
import app.photovault.android.ui.people.PersonViewModel
import app.photovault.android.ui.people.SuggestionsScreen
import app.photovault.android.ui.people.SuggestionsViewModel
import app.photovault.android.ui.search.ListCore
import app.photovault.android.ui.search.ResultLists
import app.photovault.android.ui.search.SearchResultsScreen
import app.photovault.android.ui.search.SearchResultsViewModel
import app.photovault.android.ui.search.SearchScreen
import app.photovault.android.ui.search.SearchViewModel
import app.photovault.android.ui.viewer.ViewerScreen
import app.photovault.android.ui.viewer.ViewerViewModel

/**
 * Search, people and the viewer over their photo lists (contract v1.2), added to [AppNav]'s NavHost.
 * View models get [AppGraph.ml]; they call its plain functions on Dispatchers.IO.
 */
fun NavGraphBuilder.mlDestinations(graph: AppGraph, nav: NavHostController) {
    composable<SearchRoute> {
        val context = LocalContext.current
        val vm = viewModel { SearchViewModel(graph.ml, onMlEnabled = { MlWork.scheduler(context).reschedule() }) }
        SearchScreen(
            vm = vm,
            onBack = { nav.popBackStack() },
            onOpen = { nav.openSuggestion(it) },
            onPeople = { nav.navigate(PeopleRoute) },
        )
    }
    composable<SearchResultsRoute> { entry ->
        val route = entry.toRoute<SearchResultsRoute>()
        val kind = SearchKind.entries.firstOrNull { it.name == route.kind } ?: SearchKind.Magic
        val vm = viewModel { SearchResultsViewModel(graph.ml, kind, route.key, graph.timelineLabels()) }
        val handle = entry.savedStateHandle
        val returned = remember(handle) { handle.getStateFlow<Int?>(NavResults.VIEWER_INDEX, null) }
        CompositionLocalProvider(LocalNavAnimatedScope provides this) {
            SearchResultsScreen(
                vm = vm,
                title = route.label,
                returnIndex = returned,
                onReturnConsumed = { handle[NavResults.VIEWER_INDEX] = null },
                onBack = { nav.popBackStack() },
                onOpen = { index, fileId -> nav.navigate(ListViewerRoute(vm.source, index, fileId)) },
            )
        }
    }
    composable<PeopleRoute> {
        val vm = viewModel { PeopleViewModel(graph.ml) }
        PeopleScreen(vm, onBack = { nav.popBackStack() }, onPerson = { nav.navigate(PersonRoute(it)) })
    }
    composable<PersonRoute> { entry ->
        val route = entry.toRoute<PersonRoute>()
        val vm = viewModel { PersonViewModel(graph.ml, route.personId, graph.timelineLabels()) }
        val handle = entry.savedStateHandle
        val returned = remember(handle) { handle.getStateFlow<Int?>(NavResults.VIEWER_INDEX, null) }
        val changed = remember(handle) { handle.getStateFlow<String?>(MlNavResults.PERSON_ID, null) }
        LaunchedEffect(changed) {
            changed.collect { id ->
                if (id == null) return@collect
                handle[MlNavResults.PERSON_ID] = null
                vm.onPersonChanged(id)
            }
        }
        CompositionLocalProvider(LocalNavAnimatedScope provides this) {
            PersonScreen(
                vm = vm,
                returnIndex = returned,
                onReturnConsumed = { handle[NavResults.VIEWER_INDEX] = null },
                onBack = { nav.popBackStack() },
                onOpen = { source, index, fileId -> nav.navigate(ListViewerRoute(source, index, fileId)) },
                onName = { nav.navigate(NamePersonRoute(it)) },
                onMerge = { nav.navigate(MergePersonRoute(it)) },
                onSuggestions = { nav.navigate(PersonSuggestionsRoute(it)) },
            )
        }
    }
    composable<NamePersonRoute> { entry ->
        val route = entry.toRoute<NamePersonRoute>()
        val vm = viewModel { NamePersonViewModel(graph.ml, route.personId) }
        NamePersonScreen(vm, onBack = { nav.popBackStack() }, onDone = { nav.returnPerson(it) })
    }
    composable<MergePersonRoute> { entry ->
        val route = entry.toRoute<MergePersonRoute>()
        val vm = viewModel { MergePeopleViewModel(graph.ml, route.personId) }
        MergePeopleScreen(vm, onBack = { nav.popBackStack() }, onDone = { nav.returnPerson(it) })
    }
    composable<PersonSuggestionsRoute> { entry ->
        val route = entry.toRoute<PersonSuggestionsRoute>()
        val vm = viewModel { SuggestionsViewModel(graph.ml, route.personId) }
        SuggestionsScreen(vm, onBack = { nav.popBackStack() })
    }
    composable<ListViewerRoute> { entry ->
        val route = entry.toRoute<ListViewerRoute>()
        val vm = viewModel {
            // The timeline's viewer over this list: same paging store, same decrypted-originals cache.
            val store = TimelineStore(ListCore(graph.core) { ResultLists.load(graph.ml, route.source) })
            ViewerViewModel(graph.core, store, graph.viewerCache, cleanupScope = graph.appScope, startIndex = route.index)
        }
        CompositionLocalProvider(LocalNavAnimatedScope provides this) {
            ViewerScreen(
                vm = vm,
                startIndex = route.index,
                onClose = { index ->
                    nav.previousBackStackEntry?.savedStateHandle?.set(NavResults.VIEWER_INDEX, index)
                    nav.popBackStack()
                },
            )
        }
    }
}

/** A person suggestion opens the person; everything else its results. */
private fun NavHostController.openSuggestion(s: SearchSuggestion) {
    if (s.kind == SearchKind.Person) navigate(PersonRoute(s.key))
    else navigate(SearchResultsRoute(s.kind.name, s.key, s.label))
}

/** Back to the person screen, which now shows [personId]. */
private fun NavHostController.returnPerson(personId: String) {
    previousBackStackEntry?.savedStateHandle?.set(MlNavResults.PERSON_ID, personId)
    popBackStack()
}
