package app.photovault.android.data

import android.app.ActivityManager
import android.content.Context
import app.photovault.android.data.fake.FakeCore
import app.photovault.android.data.fake.FakeSignupRepository
import app.photovault.core.Core
import app.photovault.core.CoreException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext

/** Adapter from core's sign-up exports (contract v1.2) to [SignupRepository]. */
internal class RealSignupRepository(
    private val core: Core,
    private val lowMemory: () -> Boolean,
) : SignupRepository {

    override suspend fun sendCode(email: String) = guard { core.signupSendCode(email) }

    override suspend fun verifyCode(email: String, code: String) = guard { core.signupVerifyCode(email, code) }

    override suspend fun prepare(password: String): List<String> {
        val low = withContext(Dispatchers.Default) { lowMemory() }
        return guard { core.signupPrepare(password, low) }.split(' ').filter { it.isNotEmpty() }
    }

    override suspend fun finish(): Account = guard {
        val a = core.signupFinish()
        Account(a.userId, a.email, a.serverUrl)
    }

    private suspend inline fun <T> guard(crossinline block: suspend () -> T): T =
        try { block() } catch (e: CoreException) { throw e.toApp() }
}

/** Picks the sign-up implementation that matches the [CoreRepository] in use. */
object SignupProvider {
    fun create(context: Context, core: CoreRepository): SignupRepository = when (core) {
        is RealCore -> RealSignupRepository(core.core) { isLowMemoryDevice(context) }
        is FakeCore -> FakeSignupRepository(core)
        else -> UnavailableSignupRepository()
    }

    /**
     * Ente's low-spec rule for the Argon2id start (256 MiB instead of 1 GiB): a low-RAM device, under 4 GiB
     * in total, or under 1.5 GiB free right now. Core still halves the memory if the derivation fails.
     */
    private fun isLowMemoryDevice(context: Context): Boolean {
        val am = context.getSystemService(ActivityManager::class.java) ?: return true
        val info = ActivityManager.MemoryInfo().also(am::getMemoryInfo)
        return am.isLowRamDevice || info.totalMem < 4L * GIB || info.availMem < 3L * GIB / 2
    }

    private const val GIB = 1024L * 1024L * 1024L
}
