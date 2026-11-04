package app.photovault.android.data

/**
 * Create account on the active server (contracts/photovault-core.md, "v1.2: signup"), exactly as the stock
 * apps do it: email code, keys made on the device, the recovery key shown, then the keys and SRP uploaded and
 * the account signed in. Failures are thrown as [CoreError]: `Server(403)` when the server does not accept
 * new accounts, `Server(409)` when the email already has one, [CoreError.IncorrectCode] for a wrong code.
 *
 * Every function is main-safe (core runs its work on its own runtime).
 */
interface SignupRepository {
    /** Emails a sign-up code to [email]. */
    suspend fun sendCode(email: String)

    /** Checks the emailed code. */
    suspend fun verifyCode(email: String, code: String)

    /**
     * Makes the account's keys from [password] on the device (Argon2id: seconds of work and up to 1 GiB of
     * memory) and returns the recovery key as 24 words. Nothing is uploaded yet.
     */
    suspend fun prepare(password: String): List<String>

    /** After the user saved the recovery key: uploads the keys, sets up SRP and signs in. */
    suspend fun finish(): Account
}

/** Stands in when photovault-core could not be loaded (release builds): every call reports it. */
class UnavailableSignupRepository : SignupRepository {
    private fun fail(): Nothing = throw CoreError.Storage("This app's encryption library could not be loaded.")
    override suspend fun sendCode(email: String): Unit = fail()
    override suspend fun verifyCode(email: String, code: String): Unit = fail()
    override suspend fun prepare(password: String): List<String> = fail()
    override suspend fun finish(): Account = fail()
}
