package app.photovault.android.widget

import android.app.Activity
import android.appwidget.AppWidgetManager
import android.content.Intent
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawingPadding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.getValue
import androidx.compose.runtime.produceState
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import androidx.lifecycle.lifecycleScope
import app.photovault.android.R
import app.photovault.android.appGraph
import app.photovault.android.data.PhotoCollection
import app.photovault.android.ui.image.Thumb
import app.photovault.android.ui.theme.EncoreRadius
import app.photovault.android.ui.theme.PhotovaultTheme
import app.photovault.android.ui.theme.Spacing
import app.photovault.android.ui.theme.palette
import coil3.compose.AsyncImage
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

/** Shown when an Album widget is placed (or reconfigured): pick which album it shows. */
class AlbumPickerActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        enableEdgeToEdge()
        super.onCreate(savedInstanceState)
        val widgetId = intent?.extras?.getInt(AppWidgetManager.EXTRA_APPWIDGET_ID, AppWidgetManager.INVALID_APPWIDGET_ID)
            ?: AppWidgetManager.INVALID_APPWIDGET_ID
        // Backing out cancels the placement.
        setResult(Activity.RESULT_CANCELED, Intent().putExtra(AppWidgetManager.EXTRA_APPWIDGET_ID, widgetId))
        if (widgetId == AppWidgetManager.INVALID_APPWIDGET_ID) {
            finish()
            return
        }
        val graph = appGraph
        setContent {
            PhotovaultTheme {
                val p = MaterialTheme.palette
                val albums by produceState<List<PhotoCollection>?>(null) {
                    value = withContext(Dispatchers.IO) {
                        if (graph.core.currentAccount() == null) emptyList() else graph.core.collections().filter { it.count > 0 }
                    }
                }
                Column(Modifier.fillMaxSize().background(p.bg).safeDrawingPadding().padding(Spacing.xl)) {
                    Text(stringResource(R.string.widget_pick_album), style = MaterialTheme.typography.titleLarge, color = p.text)
                    Spacer(Modifier.size(Spacing.lg))
                    val list = albums
                    when {
                        list == null -> Unit
                        list.isEmpty() -> Text(stringResource(R.string.widget_pick_album_empty), color = p.muted)
                        else -> LazyColumn(
                            verticalArrangement = Arrangement.spacedBy(Spacing.xs),
                            modifier = Modifier.testTag("widget_albums"),
                        ) {
                            items(list, key = { it.id }, contentType = { "row" }) { album ->
                                Row(
                                    Modifier
                                        .fillMaxWidth()
                                        .clip(RoundedCornerShape(EncoreRadius.tile))
                                        .background(p.panel)
                                        .clickable { choose(widgetId, album.id) }
                                        .padding(Spacing.md),
                                    verticalAlignment = Alignment.CenterVertically,
                                ) {
                                    AsyncImage(
                                        model = album.coverFileId?.let(::Thumb),
                                        contentDescription = null,
                                        contentScale = ContentScale.Crop,
                                        modifier = Modifier.size(48.dp).clip(RoundedCornerShape(EncoreRadius.small)).background(p.panel2),
                                    )
                                    Spacer(Modifier.width(Spacing.lg))
                                    Column {
                                        Text(album.name, style = MaterialTheme.typography.bodyLarge, color = p.text, maxLines = 1)
                                        Text(album.count.toString(), style = MaterialTheme.typography.bodySmall, color = p.muted)
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    private fun choose(widgetId: Int, albumId: Long) {
        WidgetPrefs.setAlbum(this, widgetId, albumId)
        lifecycleScope.launch {
            val manager = AppWidgetManager.getInstance(this@AlbumPickerActivity)
            withContext(Dispatchers.IO) { AlbumWidget().update(applicationContext, manager, widgetId) }
            setResult(Activity.RESULT_OK, Intent().putExtra(AppWidgetManager.EXTRA_APPWIDGET_ID, widgetId))
            finish()
        }
    }
}
