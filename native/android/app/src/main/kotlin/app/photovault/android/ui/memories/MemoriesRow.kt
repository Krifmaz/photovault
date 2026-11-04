package app.photovault.android.ui.memories

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyRow
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.drawWithContent
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import app.photovault.android.data.ml.Memory
import app.photovault.android.ui.image.Thumb
import app.photovault.android.ui.theme.Spacing
import app.photovault.android.ui.theme.palette
import coil3.compose.AsyncImage

/** Card size: Ente's memory card aspect ratio (29:42, memory_card_constants.dart) at a strip-friendly height. */
private val CardWidth = 104.dp
private val CardHeight = CardWidth * 42f / 29f
private val CardRadius = 14.dp

/** Scrim behind the title: two flat stops, drawn once per card (no blur, no shadow). */
private val TitleScrim = Brush.verticalGradient(0.45f to Color.Transparent, 1f to Color.Black.copy(alpha = 0.65f))

/**
 * The strip of memory covers above the timeline grid: unseen first, seen ones dimmed (Ente's memories strip).
 * Placed as the first full-span item of the grid, so it scrolls with the photos and costs nothing once
 * scrolled away. Callers only compose it when [memories] is not empty.
 *
 * Based on Ente (ente.com): mobile/apps/photos/lib/ui/home/memories/memories_strip.dart, memory_card.dart.
 */
@Composable
fun MemoriesRow(memories: List<Memory>, onOpen: (Memory) -> Unit, modifier: Modifier = Modifier) {
    LazyRow(
        modifier = modifier
            .fillMaxWidth()
            .testTag("memories_row"),
        contentPadding = PaddingValues(horizontal = Spacing.lg, vertical = Spacing.md),
        horizontalArrangement = Arrangement.spacedBy(Spacing.sm),
    ) {
        items(memories, key = { it.id }, contentType = { 0 }) { memory ->
            MemoryCover(memory, onOpen)
        }
    }
}

@Composable
private fun MemoryCover(memory: Memory, onOpen: (Memory) -> Unit) {
    val p = MaterialTheme.palette
    val shape = RoundedCornerShape(CardRadius)
    Box(
        Modifier
            .width(CardWidth)
            .height(CardHeight)
            .clip(shape)
            .background(p.panel)
            .then(if (memory.seen) Modifier else Modifier.border(1.5.dp, p.accent2, shape))
            .clickable(role = Role.Button, onClickLabel = memory.title) { onOpen(memory) }
            .testTag("memory_cover"),
    ) {
        AsyncImage(
            model = Thumb(memory.coverFileId),
            contentDescription = null,
            contentScale = ContentScale.Crop,
            modifier = Modifier
                .fillMaxSize()
                .drawWithContent {
                    drawContent()
                    drawRect(TitleScrim)
                    // seen: dimmed like Ente's foregroundDecoration
                    if (memory.seen) drawRect(Color.Black.copy(alpha = 0.45f))
                },
        )
        Column(
            Modifier
                .align(Alignment.BottomStart)
                .padding(horizontal = Spacing.md, vertical = Spacing.md),
        ) {
            Text(
                memory.title,
                style = MaterialTheme.typography.labelLarge,
                color = if (memory.seen) Color.White.copy(alpha = 0.6f) else Color.White,
                maxLines = 3,
                overflow = TextOverflow.Ellipsis,
            )
        }
    }
}
