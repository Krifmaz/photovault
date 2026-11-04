package app.photovault.android.widget

import android.app.PendingIntent
import android.appwidget.AppWidgetManager
import android.appwidget.AppWidgetProvider
import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.util.Log
import android.view.View
import android.widget.RemoteViews
import app.photovault.android.MainActivity
import app.photovault.android.R
import app.photovault.android.appGraph
import app.photovault.android.data.CoreRepository
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.launch
import kotlinx.coroutines.withTimeoutOrNull
import java.util.TimeZone

/**
 * Home-screen widgets in plain RemoteViews (no Glance, no WorkManager, nothing from Google): the system
 * refreshes them every [WidgetRefresh.PERIOD_MS] (the widget XML's updatePeriodMillis), and the app refreshes
 * them after sign-in and sign-out ([WidgetRefresh.all]).
 *
 * The photo is the thumbnail core decrypts (core's cache stays encrypted on disk); the decoded bitmap goes
 * straight into the RemoteViews and is never written to a file by the app.
 */
abstract class PhotoWidgetProvider : AppWidgetProvider() {

    protected abstract fun pick(context: Context, core: CoreRepository, widgetId: Int, slot: Long): WidgetPick

    override fun onUpdate(context: Context, manager: AppWidgetManager, ids: IntArray) {
        val pending = goAsync()
        scope.launch {
            try {
                // a broadcast has ~10 s; leave room for the system
                withTimeoutOrNull(8_000) { for (id in ids) update(context, manager, id) }
            } finally {
                pending.finish()
            }
        }
    }

    internal suspend fun update(context: Context, manager: AppWidgetManager, id: Int) {
        val views = RemoteViews(context.packageName, R.layout.widget_photo)
        views.setOnClickPendingIntent(R.id.widget_root, openApp(context))
        val pick = try {
            val core = context.appGraph.core
            pick(context, core, id, WidgetRefresh.slot(System.currentTimeMillis()))
        } catch (e: Exception) {
            Log.w(TAG, "widget $id: ${e.message}")
            WidgetPick.Empty
        }
        when (pick) {
            is WidgetPick.Photo -> {
                val bitmap = try {
                    decode(context.appGraph.core.thumbnail(pick.item.id))
                } catch (e: Exception) {
                    Log.w(TAG, "widget $id thumbnail: ${e.message}")
                    null
                }
                if (bitmap == null) {
                    // offline and not cached: keep what the widget shows rather than blanking it
                    return
                }
                views.setImageViewBitmap(R.id.widget_image, bitmap)
                views.setViewVisibility(R.id.widget_image, View.VISIBLE)
                views.setViewVisibility(R.id.widget_message, View.GONE)
                views.setTextViewText(R.id.widget_caption, caption(context, pick.caption))
                views.setViewVisibility(R.id.widget_caption, View.VISIBLE)
            }
            else -> {
                val text = when (pick) {
                    WidgetPick.SignedOut -> R.string.widget_sign_in
                    WidgetPick.AlbumMissing -> R.string.widget_album_missing
                    else -> R.string.widget_no_photos
                }
                views.setImageViewBitmap(R.id.widget_image, null)
                views.setViewVisibility(R.id.widget_image, View.GONE)
                views.setViewVisibility(R.id.widget_caption, View.GONE)
                views.setTextViewText(R.id.widget_message, context.getString(text))
                views.setViewVisibility(R.id.widget_message, View.VISIBLE)
            }
        }
        manager.updateAppWidget(id, views)
    }

    private fun caption(context: Context, c: Caption): String = when (c) {
        is Caption.YearsAgo -> if (c.years == 1) context.getString(R.string.widget_one_year_ago) else context.getString(R.string.widget_years_ago, c.years)
        Caption.Recent -> context.getString(R.string.widget_recent)
        is Caption.Album -> c.name
    }

    private fun openApp(context: Context): PendingIntent =
        PendingIntent.getActivity(
            context,
            0,
            Intent(context, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )

    companion object {
        private const val TAG = "PhotovaultWidget"
        /** Longest side of the widget bitmap: sharp on a 2x2 to 4x4 widget, small enough for RemoteViews' limits. */
        const val MAX_PX = 640
        internal val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)

        internal fun decode(bytes: ByteArray): Bitmap? {
            val bounds = BitmapFactory.Options().apply { inJustDecodeBounds = true }
            BitmapFactory.decodeByteArray(bytes, 0, bytes.size, bounds)
            var sample = 1
            while (maxOf(bounds.outWidth, bounds.outHeight) / (sample * 2) >= MAX_PX) sample *= 2
            return BitmapFactory.decodeByteArray(bytes, 0, bytes.size, BitmapFactory.Options().apply { inSampleSize = sample })
        }
    }
}

/** "On this day": a photo from today's date in earlier years, else a recent one. */
class MemoriesWidget : PhotoWidgetProvider() {
    override fun pick(context: Context, core: CoreRepository, widgetId: Int, slot: Long): WidgetPick {
        val now = System.currentTimeMillis()
        val offset = TimeZone.getDefault().getOffset(now) / 1000
        // each widget starts at a different photo so two of them do not show the same one
        return WidgetPicks.memories(core, now, offset, slot + widgetId)
    }
}

/** Photos from one album, chosen when the widget is placed ([AlbumPickerActivity]). */
class AlbumWidget : PhotoWidgetProvider() {
    override fun pick(context: Context, core: CoreRepository, widgetId: Int, slot: Long): WidgetPick {
        val album = WidgetPrefs.album(context, widgetId) ?: return WidgetPick.AlbumMissing
        return WidgetPicks.album(core, album, slot + widgetId)
    }

    override fun onDeleted(context: Context, ids: IntArray) {
        ids.forEach { WidgetPrefs.clear(context, it) }
    }
}

object WidgetPrefs {
    private const val FILE = "widgets"
    private fun prefs(context: Context) = context.getSharedPreferences(FILE, Context.MODE_PRIVATE)

    fun album(context: Context, widgetId: Int): Long? =
        prefs(context).getLong("album_$widgetId", Long.MIN_VALUE).takeIf { it != Long.MIN_VALUE }

    fun setAlbum(context: Context, widgetId: Int, albumId: Long) {
        prefs(context).edit().putLong("album_$widgetId", albumId).apply()
    }

    fun clear(context: Context, widgetId: Int) {
        prefs(context).edit().remove("album_$widgetId").apply()
    }
}

object WidgetRefresh {
    /** Matches updatePeriodMillis in res/xml/widget_*.xml. */
    const val PERIOD_MS = 30L * 60 * 1000

    fun slot(nowMs: Long): Long = nowMs / PERIOD_MS

    /** Re-renders every placed widget (after sign-in, sign-out or a sync). */
    fun all(context: Context) {
        val app = context.applicationContext
        val manager = AppWidgetManager.getInstance(app) ?: return
        for (cls in listOf(MemoriesWidget::class.java, AlbumWidget::class.java)) {
            val ids = manager.getAppWidgetIds(ComponentName(app, cls))
            if (ids.isEmpty()) continue
            app.sendBroadcast(
                Intent(app, cls).setAction(AppWidgetManager.ACTION_APPWIDGET_UPDATE)
                    .putExtra(AppWidgetManager.EXTRA_APPWIDGET_IDS, ids),
            )
        }
    }
}
