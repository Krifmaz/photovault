package app.photovault.android.ui.search

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.grid.GridCells
import androidx.compose.foundation.lazy.grid.GridItemSpan
import androidx.compose.foundation.lazy.grid.LazyGridScope
import androidx.compose.foundation.lazy.grid.LazyGridState
import androidx.compose.foundation.lazy.grid.LazyVerticalGrid
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.unit.dp
import app.photovault.android.R
import app.photovault.android.ui.theme.Spacing
import app.photovault.android.ui.theme.palette
import app.photovault.android.ui.timeline.GridContentType
import app.photovault.android.ui.timeline.GridEntry
import app.photovault.android.ui.timeline.GridModel
import app.photovault.android.ui.timeline.HeaderRow
import app.photovault.android.ui.timeline.PhotoCell
import kotlinx.coroutines.flow.Flow

/**
 * A photo list in the timeline's cells and headers ([PhotoCell], [HeaderRow]), keyed by file id so the grid
 * keeps its place when the list changes. [top] adds full-width items above the photos (a person's header).
 *
 * With [selected] non-null the grid is in selection mode: a tap toggles a photo instead of opening it.
 */
@Composable
fun ResultsGrid(
    model: GridModel,
    state: LazyGridState,
    contentPadding: PaddingValues,
    onOpen: (index: Int, fileId: Long) -> Unit,
    modifier: Modifier = Modifier,
    selected: Set<Long>? = null,
    onToggle: (Long) -> Unit = {},
    top: (LazyGridScope.() -> Unit)? = null,
) {
    val entries = model.entries
    LazyVerticalGrid(
        columns = GridCells.Fixed(model.columns),
        state = state,
        contentPadding = contentPadding,
        modifier = modifier.fillMaxSize().testTag("results_grid"),
    ) {
        top?.invoke(this)
        items(
            count = entries.size,
            key = { entries[it].key },
            span = { if (entries[it] is GridEntry.Header) GridItemSpan(maxLineSpan) else GridItemSpan(1) },
            contentType = { if (entries[it] is GridEntry.Header) GridContentType.HEADER else GridContentType.CELL },
        ) { i ->
            when (val e = entries[i]) {
                is GridEntry.Header -> HeaderRow(e.label)
                is GridEntry.Cell -> if (selected == null) {
                    PhotoCell(e, onOpen)
                } else {
                    SelectableCell(e, e.item.id in selected, onOpen, onToggle)
                }
            }
        }
    }
}

@Composable
private fun SelectableCell(cell: GridEntry.Cell, isSelected: Boolean, onOpen: (Int, Long) -> Unit, onToggle: (Long) -> Unit) {
    val p = MaterialTheme.palette
    Box {
        PhotoCell(cell, onOpen)
        // on top of the cell, so it takes the taps while selecting
        Box(
            Modifier
                .matchParentSize()
                .padding(1.dp)
                .background(if (isSelected) Color.Black.copy(alpha = 0.35f) else Color.Transparent)
                .clickable { onToggle(cell.item.id) },
        ) {
            if (isSelected) {
                Icon(
                    painterResource(R.drawable.ic_check_circle),
                    contentDescription = null,
                    tint = p.accent2,
                    modifier = Modifier.align(Alignment.TopEnd).padding(Spacing.xs).size(22.dp),
                )
            }
        }
    }
}

/**
 * Back from the viewer: brings the photo it ended on into view. [offset] is the number of items above the
 * photo entries (a header block).
 */
@Composable
fun ScrollToReturned(returnIndex: Flow<Int?>, onConsumed: () -> Unit, model: () -> GridModel, state: LazyGridState, offset: Int = 0) {
    LaunchedEffect(returnIndex) {
        returnIndex.collect { index ->
            if (index == null) return@collect
            onConsumed()
            val entry = model().entryIndexOfTimeline.getOrNull(index) ?: return@collect
            val target = entry + offset
            val visible = state.layoutInfo.visibleItemsInfo
            val first = visible.firstOrNull()?.index ?: 0
            val last = visible.lastOrNull()?.index ?: 0
            if (target !in (first + 1) until last) state.scrollToItem(target)
        }
    }
}
