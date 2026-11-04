package io.ente.photos

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.os.Handler
import android.os.Looper
import android.util.Log
import androidx.core.app.NotificationCompat
import androidx.core.content.ContextCompat
import androidx.core.content.edit
import io.flutter.plugin.common.BinaryMessenger
import io.flutter.plugin.common.MethodCall
import io.flutter.plugin.common.MethodChannel

/**
 * PHOTOVAULT: backup progress notification.
 *
 * Dart (BackupProgressService) sends what to show over
 * "app.photovault/backup_progress"; all wording comes from Dart. While a backup
 * uploads, [BackupProgressService] runs as a dataSync foreground service so
 * Android does not freeze or kill the app in the background or with the screen
 * off. Paused, failed and finished states are plain notifications (no
 * foreground service, no wake lock).
 */
object BackupProgressBridge {
    const val TAG = "BackupProgress"
    const val CHANNEL_NAME = "app.photovault/backup_progress"
    const val NOTIFICATION_CHANNEL_ID = "backup_progress"
    const val NOTIFICATION_ID = 7301
    const val ACTION_OPEN_BACKUP_STATUS = "app.photovault.action.OPEN_BACKUP_STATUS"
    const val ACTION_NOTIFICATION = "app.photovault.action.BACKUP_NOTIFICATION"
    const val EXTRA_ACTION_ID = "action_id"
    private const val PAUSED_PREF = "flutter.photovault.backup_paused"

    private val main = Handler(Looper.getMainLooper())
    private var channel: MethodChannel? = null
    private var appContext: Context? = null
    private var pendingOpenBackupStatus = false

    /** Last spec from Dart, also used by the service for startForeground. */
    @Volatile
    var current: Spec? = null
        private set

    fun attach(context: Context, messenger: BinaryMessenger) {
        appContext = context.applicationContext
        channel = MethodChannel(messenger, CHANNEL_NAME).also {
            it.setMethodCallHandler(::onMethodCall)
        }
    }

    /**
     * The Activity's engine is going away. Uploads run in that engine, so a
     * progress notification left behind would be stale.
     */
    fun detach(context: Context, finishing: Boolean) {
        channel?.setMethodCallHandler(null)
        channel = null
        if (finishing && current?.foreground == true) {
            stop(context)
        }
    }

    /** MainActivity got an intent; true when it was a tap on the notification. */
    fun handleIntent(intent: Intent?): Boolean {
        if (intent?.action != ACTION_OPEN_BACKUP_STATUS) return false
        val ch = channel
        if (ch != null) {
            ch.invokeMethod("openBackupStatus", null)
        } else {
            pendingOpenBackupStatus = true
        }
        return true
    }

    private fun onMethodCall(call: MethodCall, result: MethodChannel.Result) {
        val context = appContext ?: return result.error("no_context", null, null)
        when (call.method) {
            "show" -> {
                @Suppress("UNCHECKED_CAST")
                val args = call.arguments as? Map<String, Any?> ?: emptyMap()
                show(context, Spec.fromMap(args))
                result.success(null)
            }
            "consumeLaunchAction" -> {
                val pending = pendingOpenBackupStatus
                pendingOpenBackupStatus = false
                result.success(pending)
            }
            else -> result.notImplemented()
        }
    }

    fun show(context: Context, spec: Spec) {
        current = spec
        val manager = context.getSystemService(NotificationManager::class.java)
        if (spec.phase == "idle") {
            stop(context)
            return
        }
        ensureChannel(context, spec)
        if (spec.foreground) {
            if (BackupProgressService.isRunning) {
                manager.notify(NOTIFICATION_ID, build(context, spec))
                BackupProgressService.onProgress()
            } else {
                startService(context, spec)
            }
        } else {
            // Leave foreground first so the final notification is dismissable.
            BackupProgressService.stopForegroundIfRunning(context)
            manager.notify(NOTIFICATION_ID, build(context, spec))
        }
    }

    fun stop(context: Context) {
        BackupProgressService.stopForegroundIfRunning(context)
        context.getSystemService(NotificationManager::class.java).cancel(NOTIFICATION_ID)
    }

    private fun startService(context: Context, spec: Spec) {
        val manager = context.getSystemService(NotificationManager::class.java)
        try {
            ContextCompat.startForegroundService(
                context,
                Intent(context, BackupProgressService::class.java),
            )
        } catch (e: Exception) {
            // Android 12+ refuses to start a foreground service while the app
            // is in the background (ForegroundServiceStartNotAllowedException).
            // Keep the user informed anyway.
            Log.w(TAG, "foreground service not started", e)
            manager.notify(NOTIFICATION_ID, build(context, spec))
        }
    }

    fun ensureChannel(context: Context, spec: Spec?) {
        val manager = context.getSystemService(NotificationManager::class.java)
        val name = spec?.channelName ?: "Backup progress"
        val channel =
            NotificationChannel(
                NOTIFICATION_CHANNEL_ID,
                name,
                NotificationManager.IMPORTANCE_LOW,
            ).apply {
                description = spec?.channelDescription
                setSound(null, null)
                enableVibration(false)
                enableLights(false)
                setShowBadge(false)
                lockscreenVisibility = Notification.VISIBILITY_PRIVATE
            }
        manager.createNotificationChannel(channel)
    }

    fun build(context: Context, spec: Spec): Notification {
        val open =
            PendingIntent.getActivity(
                context,
                0,
                Intent(context, MainActivity::class.java)
                    .setAction(ACTION_OPEN_BACKUP_STATUS)
                    .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_SINGLE_TOP),
                PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
            )
        val builder =
            NotificationCompat.Builder(context, NOTIFICATION_CHANNEL_ID)
                .setSmallIcon(R.drawable.notification_icon)
                .setContentTitle(spec.title)
                .setContentIntent(open)
                .setOnlyAlertOnce(true)
                .setSilent(true)
                .setShowWhen(false)
                .setCategory(NotificationCompat.CATEGORY_PROGRESS)
                .setPriority(NotificationCompat.PRIORITY_LOW)
                .setOngoing(spec.foreground)
                .setAutoCancel(!spec.foreground)
                .setLocalOnly(true)
                .setVisibility(NotificationCompat.VISIBILITY_PRIVATE)
                .setForegroundServiceBehavior(NotificationCompat.FOREGROUND_SERVICE_IMMEDIATE)
        spec.text?.takeIf { it.isNotEmpty() }?.let { builder.setContentText(it) }
        spec.subText?.takeIf { it.isNotEmpty() }?.let { builder.setSubText(it) }
        spec.bigText?.takeIf { it.isNotEmpty() }?.let {
            builder.setStyle(NotificationCompat.BigTextStyle().bigText(it))
        }
        if (spec.showProgress) {
            builder.setProgress(spec.max.coerceAtLeast(0), spec.progress.coerceAtLeast(0), spec.indeterminate)
        }
        if (spec.autoDismissMs > 0) {
            builder.setTimeoutAfter(spec.autoDismissMs)
        }
        spec.actions.forEachIndexed { index, (id, label) ->
            val intent =
                Intent(context, BackupProgressActionReceiver::class.java)
                    .setAction(ACTION_NOTIFICATION)
                    .putExtra(EXTRA_ACTION_ID, id)
            val pending =
                PendingIntent.getBroadcast(
                    context,
                    100 + index,
                    intent,
                    PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
                )
            builder.addAction(0, label, pending)
        }
        return builder.build()
    }

    /** A tap on Pause, Resume, Retry or Wi-Fi only. */
    fun onAction(context: Context, id: String) {
        main.post {
            val ch = channel
            if (id == "resume" || id == "retry") {
                // Start the foreground service now: a notification action
                // lets the app do that from the background, Dart may be late.
                current?.let { spec ->
                    val resuming =
                        spec.copy(
                            phase = "uploading",
                            title = spec.resumingTitle ?: spec.title,
                            text = null,
                            subText = null,
                            bigText = null,
                            showProgress = true,
                            indeterminate = true,
                            foreground = true,
                            autoDismissMs = 0,
                            actions = emptyList(),
                        )
                    show(context, resuming)
                }
            }
            if (ch != null) {
                ch.invokeMethod(id, null)
                return@post
            }
            // No Flutter engine: nothing is uploading. Remember a pause for
            // the next start and clear the stale notification.
            if (id == "pause") {
                context
                    .getSharedPreferences(EnteApplication.FLUTTER_SHARED_PREFERENCES, Context.MODE_PRIVATE)
                    .edit { putBoolean(PAUSED_PREF, true) }
            }
            stop(context)
        }
    }

    data class Spec(
        val phase: String,
        val title: String,
        val text: String?,
        val subText: String?,
        val bigText: String?,
        val progress: Int,
        val max: Int,
        val indeterminate: Boolean,
        val showProgress: Boolean,
        val foreground: Boolean,
        val autoDismissMs: Long,
        val actions: List<Pair<String, String>>,
        val channelName: String?,
        val channelDescription: String?,
        val resumingTitle: String?,
    ) {
        companion object {
            fun fromMap(map: Map<String, Any?>): Spec {
                val actions =
                    (map["actions"] as? List<*>).orEmpty().mapNotNull { raw ->
                        val action = raw as? Map<*, *> ?: return@mapNotNull null
                        val id = action["id"] as? String ?: return@mapNotNull null
                        id to (action["label"] as? String ?: id)
                    }
                return Spec(
                    phase = map["phase"] as? String ?: "idle",
                    title = map["title"] as? String ?: "",
                    text = map["text"] as? String,
                    subText = map["subText"] as? String,
                    bigText = map["bigText"] as? String,
                    progress = (map["progress"] as? Number)?.toInt() ?: 0,
                    max = (map["max"] as? Number)?.toInt() ?: 0,
                    indeterminate = map["indeterminate"] as? Boolean ?: false,
                    showProgress = map["showProgress"] as? Boolean ?: false,
                    foreground = map["foreground"] as? Boolean ?: false,
                    autoDismissMs = (map["autoDismissMs"] as? Number)?.toLong() ?: 0L,
                    actions = actions,
                    channelName = map["channelName"] as? String,
                    channelDescription = map["channelDescription"] as? String,
                    resumingTitle = map["resumingTitle"] as? String,
                )
            }
        }
    }
}

class BackupProgressActionReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        if (intent.action != BackupProgressBridge.ACTION_NOTIFICATION) return
        val id = intent.getStringExtra(BackupProgressBridge.EXTRA_ACTION_ID) ?: return
        BackupProgressBridge.onAction(context.applicationContext, id)
    }
}
