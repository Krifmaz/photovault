package app.photovault.android.backup

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.os.Build
import android.text.format.Formatter
import androidx.work.ForegroundInfo
import app.photovault.android.MainActivity
import app.photovault.android.R
import app.photovault.android.data.UploadStatus

/** The ongoing "Backing up" notification of a running backup (foreground service type dataSync). */
class BackupNotifications(private val context: Context) {

    fun ensureChannel() {
        val nm = context.getSystemService(NotificationManager::class.java) ?: return
        if (nm.getNotificationChannel(CHANNEL) != null) return
        nm.createNotificationChannel(
            NotificationChannel(CHANNEL, context.getString(R.string.backup_channel), NotificationManager.IMPORTANCE_LOW).apply {
                description = context.getString(R.string.backup_channel_description)
                setShowBadge(false)
            },
        )
    }

    fun foregroundInfo(status: UploadStatus?): ForegroundInfo {
        ensureChannel()
        val n = notification(status)
        return if (Build.VERSION.SDK_INT >= 29) ForegroundInfo(ID, n, ServiceInfo.FOREGROUND_SERVICE_TYPE_DATA_SYNC)
        else ForegroundInfo(ID, n)
    }

    private fun notification(status: UploadStatus?): Notification {
        val open = PendingIntent.getActivity(
            context,
            0,
            Intent(context, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_SINGLE_TOP),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
        val b = Notification.Builder(context, CHANNEL)
            .setSmallIcon(R.drawable.ic_backup)
            .setContentTitle(context.getString(R.string.backup_notification_title))
            .setContentIntent(open)
            .setOngoing(true)
            .setOnlyAlertOnce(true)
            .setCategory(Notification.CATEGORY_PROGRESS)
        if (status == null || status.bytesTotal <= 0) {
            b.setContentText(context.getString(R.string.backup_notification_preparing)).setProgress(0, 0, true)
        } else {
            val left = status.pending + status.uploading
            b.setContentText(
                context.getString(
                    R.string.backup_notification_progress,
                    left,
                    Formatter.formatShortFileSize(context, status.bytesDone),
                    Formatter.formatShortFileSize(context, status.bytesTotal),
                ),
            )
            b.setProgress(PROGRESS_MAX, progressOf(status), false)
        }
        if (Build.VERSION.SDK_INT >= 31) b.setForegroundServiceBehavior(Notification.FOREGROUND_SERVICE_IMMEDIATE)
        return b.build()
    }

    companion object {
        const val CHANNEL = "backup"
        const val ID = 4201
        const val PROGRESS_MAX = 1000

        fun progressOf(s: UploadStatus): Int =
            if (s.bytesTotal <= 0) 0 else ((s.bytesDone.coerceAtMost(s.bytesTotal) * PROGRESS_MAX) / s.bytesTotal).toInt()
    }
}
