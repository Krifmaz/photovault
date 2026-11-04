package app.photovault.android.data

import android.content.Context
import android.util.Log
import app.photovault.android.BuildConfig
import app.photovault.android.data.fake.FakeCore
import app.photovault.android.data.ml.MlRepository
import app.photovault.android.data.ml.RealMlRepository
import app.photovault.android.data.ml.UnavailableMlRepository
import app.photovault.android.data.ml.fake.FakeMlRepository
import app.photovault.core.Core
import app.photovault.core.CoreException
import java.io.File
import java.io.IOException
import java.security.GeneralSecurityException

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
            // The store's key never touches the disk in the clear: it is kept wrapped by the Android Keystore.
            val key = DeviceKey.load(data)
            try {
                RealCore(Core.withDeviceKey(data.path, cache.path, key))
            } finally {
                key.fill(0)
            }
        } catch (e: CoreException) {
            fallback(e.toApp().message ?: e.javaClass.simpleName, e)
        } catch (e: LinkageError) {
            // UnsatisfiedLinkError (no .so for this ABI) or ExceptionInInitializerError / NoClassDefFoundError
            // from JNA's registration of the library.
            fallback(e.message ?: e.javaClass.simpleName, e)
        } catch (e: GeneralSecurityException) {
            fallback("the Android Keystore is not usable: ${e.message ?: e.javaClass.simpleName}", e)
        } catch (e: IOException) {
            fallback(e.message ?: e.javaClass.simpleName, e)
        }
    }

    /**
     * Search, people and memories for the [CoreRepository] [create] returned: the same real core, the demo
     * library's fake, or an unavailable stand-in that reports the same problem.
     */
    fun createMl(core: CoreRepository): MlRepository = when (core) {
        is RealCore -> RealMlRepository(core.core)
        is UnavailableCore -> UnavailableMlRepository(core.reason)
        else -> FakeMlRepository(core)
    }

    private fun fallback(reason: String, cause: Throwable): CoreRepository {
        Log.e(TAG, "photovault-core is not available: $reason", cause)
        return if (BuildConfig.DEBUG) FakeCore() else UnavailableCore(reason)
    }
}
