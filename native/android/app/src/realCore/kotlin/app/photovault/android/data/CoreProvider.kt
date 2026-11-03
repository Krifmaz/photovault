package app.photovault.android.data

import android.content.Context
import android.util.Log
import app.photovault.android.BuildConfig
import app.photovault.android.data.fake.FakeCore
import app.photovault.core.Core
import app.photovault.core.CoreException
import java.io.File

/**
 * Built with photovault-core: the real UniFFI core, chosen at runtime once libphotovault_core.so loads.
 *
 * - `-Pphotovault.demo=true` (BuildConfig.DEMO) forces the in-memory demo library.
 * - If the native library cannot be loaded (wrong ABI, missing .so) or core cannot open its store, a debug
 *   build falls back to the demo library; a release build gets [UnavailableCore], which reports the
 *   problem on the connect screen instead of showing fake photos.
 *
 * Opening core decrypts the local timeline index, so [create] must be called off the main thread
 * (AppGraph warms it up on a background thread at process start).
 */
object CoreProvider {
    private const val TAG = "PhotovaultCore"

    fun create(context: Context): CoreRepository {
        if (BuildConfig.DEMO) return FakeCore()
        return try {
            val data = File(context.noBackupFilesDir, "core").apply { mkdirs() }
            val cache = File(context.cacheDir, "core").apply { mkdirs() }
            RealCore(Core(data.path, cache.path))
        } catch (e: CoreException) {
            fallback(e.toApp().message ?: e.javaClass.simpleName, e)
        } catch (e: LinkageError) {
            // UnsatisfiedLinkError (no .so for this ABI) or ExceptionInInitializerError / NoClassDefFoundError
            // from JNA's registration of the library.
            fallback(e.message ?: e.javaClass.simpleName, e)
        }
    }

    private fun fallback(reason: String, cause: Throwable): CoreRepository {
        Log.e(TAG, "photovault-core is not available: $reason", cause)
        return if (BuildConfig.DEBUG) FakeCore() else UnavailableCore(reason)
    }
}
