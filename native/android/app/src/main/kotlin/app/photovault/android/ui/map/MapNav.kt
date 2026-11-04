package app.photovault.android.ui.map

import app.photovault.android.data.TimelineStore

/**
 * The timeline position of [fileId], loading the timeline further until it is found (the viewer pages
 * through the timeline and opens at a position). Null if the photo is not in the timeline.
 */
suspend fun TimelineStore.timelineIndexOf(fileId: Long): Int? {
    if (!loaded.value) refresh()
    while (true) {
        val list = items.value
        val i = list.indexOfFirst { it.id == fileId }
        if (i >= 0) return i
        if (list.size >= total.value) return null
        ensureLoaded(list.size + TimelineStore.CHUNK * 5)
        if (items.value.size == list.size) return null
    }
}
