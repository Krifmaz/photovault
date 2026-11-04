package io.ente.photos

import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.net.wifi.WifiManager
import android.os.Build
import android.os.IBinder
import android.os.PowerManager
import android.util.Log

/**
 * PHOTOVAULT: dataSync foreground service that keeps the app alive while a
 * backup uploads. The uploads themselves run in the app's Flutter engine; this
 * service only holds the process in the foreground and keeps the CPU and Wi-Fi
 * awake with the screen off. It shows BackupProgressBridge's notification.
 */
class BackupProgressService : Service() {
    private var wakeLock: PowerManager.WakeLock? = null
    private var wifiLock: WifiManager.WifiLock? = null

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onCreate() {
        super.onCreate()
        instance = this
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        if (intent?.action == ACTION_STOP) {
            leaveForeground()
            return START_NOT_STICKY
        }
        val spec = BackupProgressBridge.current
        BackupProgressBridge.ensureChannel(this, spec)
        val notification =
            if (spec != null) {
                BackupProgressBridge.build(this, spec)
            } else {
                BackupProgressBridge.build(this, BackupProgressBridge.Spec.fromMap(mapOf("title" to "")))
            }
        try {
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
                startForeground(
                    BackupProgressBridge.NOTIFICATION_ID,
                    notification,
                    ServiceInfo.FOREGROUND_SERVICE_TYPE_DATA_SYNC,
                )
            } else {
                startForeground(BackupProgressBridge.NOTIFICATION_ID, notification)
            }
            isRunning = true
            acquireLocks()
        } catch (e: Exception) {
            // Android 12+ may refuse when the app is in the background. The
            // bridge then shows a plain notification.
            Log.w(BackupProgressBridge.TAG, "startForeground failed", e)
            isRunning = false
            stopSelf()
            if (spec != null) {
                getSystemService(android.app.NotificationManager::class.java)
                    .notify(BackupProgressBridge.NOTIFICATION_ID, notification)
            }
            return START_NOT_STICKY
        }
        if (spec != null && !spec.foreground) {
            // Dart moved on (paused/finished) before the service came up.
            leaveForeground()
        }
        // Don't restart on our own: the uploads live in the app's engine.
        return START_NOT_STICKY
    }

    /** Android 15+: dataSync services get about 6 hours a day. */
    override fun onTimeout(startId: Int, fgsType: Int) {
        Log.w(BackupProgressBridge.TAG, "dataSync time limit reached")
        leaveForeground()
    }

    override fun onDestroy() {
        releaseLocks()
        isRunning = false
        if (instance === this) instance = null
        super.onDestroy()
    }

    private fun leaveForeground() {
        // DETACH keeps the notification Dart posts next (paused, finished).
        stopForeground(STOP_FOREGROUND_DETACH)
        isRunning = false
        releaseLocks()
        stopSelf()
    }

    private fun acquireLocks() {
        if (wakeLock?.isHeld != true) {
            wakeLock =
                getSystemService(PowerManager::class.java)
                    .newWakeLock(PowerManager.PARTIAL_WAKE_LOCK, "photovault:backup")
                    .apply {
                        setReferenceCounted(false)
                        // Safety net; refreshed while the backup keeps running.
                        acquire(WAKE_LOCK_TIMEOUT_MS)
                    }
        }
        if (wifiLock?.isHeld != true) {
            wifiLock =
                runCatching {
                    @Suppress("DEPRECATION")
                    val mode =
                        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
                            WifiManager.WIFI_MODE_FULL_HIGH_PERF
                        } else {
                            WifiManager.WIFI_MODE_FULL
                        }
                    (applicationContext.getSystemService(Context.WIFI_SERVICE) as WifiManager)
                        .createWifiLock(mode, "photovault:backup")
                        .apply {
                            setReferenceCounted(false)
                            acquire()
                        }
                }.getOrNull()
        }
    }

    internal fun refreshLocks() {
        wakeLock?.takeIf { it.isHeld }?.acquire(WAKE_LOCK_TIMEOUT_MS)
    }

    private fun releaseLocks() {
        wakeLock?.takeIf { it.isHeld }?.release()
        wakeLock = null
        wifiLock?.takeIf { it.isHeld }?.release()
        wifiLock = null
    }

    companion object {
        const val ACTION_STOP = "app.photovault.action.STOP_BACKUP_FOREGROUND"
        private const val WAKE_LOCK_TIMEOUT_MS = 30 * 60 * 1000L

        @Volatile
        var isRunning = false
            private set

        @Volatile
        private var instance: BackupProgressService? = null

        /** Called on every progress update. */
        fun onProgress() {
            instance?.refreshLocks()
        }

        fun stopForegroundIfRunning(context: Context) {
            val service = instance ?: return
            if (!isRunning) return
            service.leaveForeground()
        }
    }
}
