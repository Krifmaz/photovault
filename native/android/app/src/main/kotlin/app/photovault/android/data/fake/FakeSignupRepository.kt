package app.photovault.android.data.fake

import app.photovault.android.data.Account
import app.photovault.android.data.CoreError
import app.photovault.android.data.LoginStep
import app.photovault.android.data.SignupRepository
import kotlinx.coroutines.delay

/**
 * Demo sign-up on top of [FakeCore]: a server whose address contains "nosignup" does not accept accounts, an
 * email containing "taken" already has one, the code is [FakeCore.DEMO_CODE], and finishing signs the new
 * account in to the fake library. No keys are made.
 */
class FakeSignupRepository(
    private val core: FakeCore,
    private val latencyMs: Long = 350L,
) : SignupRepository {
    private var email: String? = null
    private var verified = false
    private var password: String? = null

    override suspend fun sendCode(email: String) {
        delay(latencyMs)
        val active = core.activeBaseUrl() ?: throw CoreError.Network("Choose a server first")
        val server = core.savedServers().firstOrNull { it.publicUrl == active || it.homeUrl == active }
        if (server != null && !server.signupsAllowed) throw CoreError.Server(403, "this server is not accepting new accounts")
        if ("taken" in email.lowercase()) throw CoreError.Server(409, "an account with this email already exists")
        this.email = email.trim()
        verified = false
    }

    override suspend fun verifyCode(email: String, code: String) {
        delay(latencyMs)
        if (email.trim() != this.email || code.trim() != FakeCore.DEMO_CODE) throw CoreError.IncorrectCode()
        verified = true
    }

    override suspend fun prepare(password: String): List<String> {
        delay(latencyMs * 3)
        if (!verified) throw CoreError.NotLoggedIn()
        this.password = password
        return DEMO_WORDS
    }

    override suspend fun finish(): Account {
        val e = email ?: throw CoreError.NotLoggedIn()
        val pw = password ?: throw CoreError.NotLoggedIn()
        val step = core.login(e, pw)
        password = null
        verified = false
        return (step as? LoginStep.Done)?.account ?: throw CoreError.Server(500, "the demo sign-in did not finish")
    }

    companion object {
        /** Display only: the demo makes no keys. */
        val DEMO_WORDS: List<String> = (
            "abandon ability able about above absent absorb abstract absurd abuse access accident " +
                "account accuse achieve acid acoustic acquire across act action actor actress actual"
            ).split(' ')
    }
}
