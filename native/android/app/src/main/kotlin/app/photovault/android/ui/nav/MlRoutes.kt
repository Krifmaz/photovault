package app.photovault.android.ui.nav

import kotlinx.serialization.Serializable

/*
 * Routes of search and people (contract v1.2). Registered by [mlDestinations] (MlNav.kt).
 */

/** Search; a self-contained destination so it can also become a tab. */
@Serializable
data object SearchRoute

/** The photos of one search suggestion. [kind] is a `SearchKind` name, [key] its opaque key. */
@Serializable
data class SearchResultsRoute(val kind: String, val key: String, val label: String)

/** All people. */
@Serializable
data object PeopleRoute

/** One person or unnamed cluster (`cluster:<id>`). */
@Serializable
data class PersonRoute(val personId: String)

/** Name a cluster / rename a person (or merge into an existing name). */
@Serializable
data class NamePersonRoute(val personId: String)

/** Merge a person with another. */
@Serializable
data class MergePersonRoute(val personId: String)

/** Review the clusters suggested for a person. */
@Serializable
data class PersonSuggestionsRoute(val personId: String)

/**
 * The viewer over a photo list that is not the timeline. [source] is a
 * [app.photovault.android.ui.search.PhotoSource] string; [index] the position in that list.
 */
@Serializable
data class ListViewerRoute(val source: String, val index: Int, val fileId: Long)

/** Results of the people screens, handed back through the previous entry's SavedStateHandle. */
object MlNavResults {
    /** The id the person now has (naming a cluster or merging changes it). */
    const val PERSON_ID = "person_id"
}
