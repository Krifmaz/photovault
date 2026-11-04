package io.ente.photos

import android.content.Intent
import android.os.Bundle
import io.ente.background.BackgroundManagerPlugin
import io.flutter.embedding.android.FlutterFragmentActivity
import io.flutter.embedding.engine.FlutterEngine
import java.io.FileDescriptor
import java.io.PrintWriter

class MainActivity : FlutterFragmentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        BackgroundManagerPlugin.onForeground()
        super.onCreate(savedInstanceState)
        // PHOTOVAULT: opened from the backup progress notification.
        BackupProgressBridge.handleIntent(intent)
    }

    override fun onNewIntent(intent: Intent) {
        // PHOTOVAULT: tap on the backup progress notification while running.
        if (BackupProgressBridge.handleIntent(intent)) {
            super.onNewIntent(intent)
            return
        }
        setIntent(intent)
        super.onNewIntent(intent)
    }

    override fun configureFlutterEngine(flutterEngine: FlutterEngine) {
        super.configureFlutterEngine(flutterEngine)
        BackupProgressBridge.attach(this, flutterEngine.dartExecutor.binaryMessenger)
    }

    override fun cleanUpFlutterEngine(flutterEngine: FlutterEngine) {
        BackupProgressBridge.detach(this, isFinishing)
        super.cleanUpFlutterEngine(flutterEngine)
    }

    override fun dump(
        prefix: String,
        fd: FileDescriptor?,
        writer: PrintWriter,
        args: Array<out String>?,
    ) {
        if (args?.singleOrNull() == "--ente-logs") {
            AdbDiagnostics.dumpLogs(this, writer)
        } else {
            super.dump(prefix, fd, writer, args)
        }
    }
}
