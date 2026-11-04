package app.photovault.android.ui.signin

import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import app.photovault.android.data.CoreError
import app.photovault.android.data.CoreRepository
import app.photovault.android.data.LoginStep
import kotlinx.coroutines.CoroutineDispatcher
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.receiveAsFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

sealed interface SignInProblem {
    data object MissingFields : SignInProblem
    data object IncorrectPassword : SignInProblem
    data object IncorrectCode : SignInProblem
    data object PasswordsDiffer : SignInProblem
    data object PasswordTooShort : SignInProblem
    /** The passkey was not confirmed in the browser yet. */
    data object PasskeyPending : SignInProblem
    data class Unreachable(val address: String) : SignInProblem
    data class Other(val message: String) : SignInProblem
}

data class SignInUiState(
    val serverLabel: String = "",
    val email: String = "",
    val busy: Boolean = false,
    val problem: SignInProblem? = null,
    /** A code was (re)sent to [email]; the code screen says so. */
    val codeSent: Boolean = false,
    /** The active server accepts new accounts (saved server info; null until read). */
    val signupsAllowed: Boolean? = null,
)

/** Where the sign-in goes next (LoginStep -> screen). */
sealed interface SignInEvent {
    data object EmailCode : SignInEvent
    data class Totp(val sessionId: String) : SignInEvent
    data class Passkey(val sessionId: String, val url: String) : SignInEvent
    data object RecoveryKey : SignInEvent
    data object Done : SignInEvent
    /** The password is gone (the process was restarted mid sign-in): start over at the sign-in screen. */
    data object Restart : SignInEvent
}

/**
 * Sign-in for the active server, shared by the email/password, email code, authenticator code and recovery
 * key screens (scoped to the auth graph). The password stays in memory only for the steps that need it
 * (core's verify_email_ott / verify_totp derive the key from it) and is dropped when done.
 */
class SignInViewModel(
    private val core: CoreRepository,
    private val io: CoroutineDispatcher = Dispatchers.IO,
) : ViewModel() {

    private val _state = MutableStateFlow(SignInUiState())
    private val events = Channel<SignInEvent>(Channel.BUFFERED)
    private var password: String = ""
    /** "Use recovery key" was chosen: the steps run without a password on purpose. */
    private var recovering = false

    val state: StateFlow<SignInUiState> = _state.asStateFlow()
    val eventFlow: Flow<SignInEvent> = events.receiveAsFlow()

    init {
        viewModelScope.launch {
            val (label, signups) = withContext(io) {
                val active = core.activeBaseUrl()
                val server = core.savedServers().firstOrNull { it.publicUrl == active || it.homeUrl == active }
                (server?.name?.takeIf { it.isNotBlank() } ?: server?.publicUrl ?: active.orEmpty()) to server?.signupsAllowed
            }
            _state.update { it.copy(serverLabel = label, signupsAllowed = signups) }
        }
    }

    fun onEmailChange(email: String) = _state.update { it.copy(email = email, problem = null) }

    fun clearProblem() = _state.update { it.copy(problem = null) }

    fun signIn(email: String, password: String) {
        val e = email.trim()
        if (e.isEmpty() || password.isEmpty()) {
            _state.update { it.copy(problem = SignInProblem.MissingFields) }
            return
        }
        this.password = password
        recovering = false
        _state.update { it.copy(email = e) }
        runStep { core.login(e, password) }
    }

    /** Sends (or re-sends) the email code. */
    fun sendCode() {
        val e = _state.value.email
        launchBusy {
            core.sendEmailOtt(e)
            _state.update { it.copy(codeSent = true) }
        }
    }

    /**
     * "Use recovery key": core unlocks with a recovery key only after the server has authorized the account,
     * so this verifies the email first (the forgot-password flow of the stock apps). The code is checked
     * without a password, which makes core answer NeedsRecoveryKey (after TOTP if the account has it) and
     * opens the recovery screen.
     */
    fun startRecovery() {
        val e = _state.value.email.trim()
        if (e.isEmpty()) return problem(SignInProblem.MissingFields)
        password = ""
        recovering = true
        _state.update { it.copy(email = e) }
        launchBusy {
            core.sendEmailOtt(e)
            _state.update { it.copy(codeSent = true) }
            events.send(SignInEvent.EmailCode)
        }
    }

    fun verifyEmailCode(code: String) {
        if (code.isBlank()) return problem(SignInProblem.MissingFields)
        if (passwordLost()) return
        runStep { core.verifyEmailOtt(_state.value.email, code.trim(), password) }
    }

    fun verifyTotp(sessionId: String, code: String) {
        if (code.isBlank()) return problem(SignInProblem.MissingFields)
        if (passwordLost()) return
        runStep { core.verifyTotp(sessionId, code.trim(), password) }
    }

    /**
     * After the passkey ceremony in the browser (the user taps "Continue", or the app comes back to the
     * front): asks core whether the session is verified. [quiet] checks (on resume) do not report "not yet".
     */
    fun checkPasskey(sessionId: String, quiet: Boolean = false) {
        if (passwordLost()) return
        launchBusy {
            val next = core.verifyPasskey(sessionId, password)
            if (next is LoginStep.Passkey && next.sessionId == sessionId) {
                if (!quiet) _state.update { it.copy(problem = SignInProblem.PasskeyPending) }
            } else {
                route(next)
            }
        }
    }

    /**
     * The password lives only in this view model, never in saved state. After process death a restored
     * code screen has none; core would take that for the forgot-password path and ask for the recovery
     * key, so a normal sign-in starts over instead.
     */
    private fun passwordLost(): Boolean {
        if (password.isNotEmpty() || recovering) return false
        viewModelScope.launch { events.send(SignInEvent.Restart) }
        return true
    }

    fun recover(recoveryKey: String, newPassword: String, confirm: String) {
        when {
            recoveryKey.isBlank() || newPassword.isEmpty() -> return problem(SignInProblem.MissingFields)
            newPassword != confirm -> return problem(SignInProblem.PasswordsDiffer)
            newPassword.length < MIN_PASSWORD -> return problem(SignInProblem.PasswordTooShort)
        }
        launchBusy {
            core.recoverWithKey(recoveryKey.trim(), newPassword)
            finish()
        }
    }

    private fun problem(p: SignInProblem) = _state.update { it.copy(problem = p) }

    /** Runs a step and routes its LoginStep. */
    private fun runStep(step: suspend () -> LoginStep) = launchBusy { route(step()) }

    private suspend fun route(next: LoginStep) {
        when (next) {
            is LoginStep.Done -> finish()
            LoginStep.EmailOtt -> {
                // core's login has already sent the code (contracts/photovault-core.md); "Send a new code"
                // on the code screen calls sendCode().
                _state.update { it.copy(codeSent = true) }
                events.send(SignInEvent.EmailCode)
            }
            is LoginStep.TwoFactorTotp -> events.send(SignInEvent.Totp(next.sessionId))
            is LoginStep.Passkey -> events.send(SignInEvent.Passkey(next.sessionId, next.url))
            LoginStep.NeedsRecoveryKey -> events.send(SignInEvent.RecoveryKey)
        }
    }

    private suspend fun finish() {
        password = ""
        recovering = false
        events.send(SignInEvent.Done)
    }

    private fun launchBusy(block: suspend () -> Unit) {
        if (_state.value.busy) return
        viewModelScope.launch {
            _state.update { it.copy(busy = true, problem = null) }
            try {
                block()
            } catch (e: CoreError) {
                _state.update { it.copy(problem = e.toProblem()) }
            } finally {
                _state.update { it.copy(busy = false) }
            }
        }
    }

    private fun CoreError.toProblem(): SignInProblem = when (this) {
        is CoreError.IncorrectPassword, is CoreError.Unauthorized -> SignInProblem.IncorrectPassword
        is CoreError.IncorrectCode -> SignInProblem.IncorrectCode
        is CoreError.Unreachable -> SignInProblem.Unreachable(address)
        else -> SignInProblem.Other(message ?: javaClass.simpleName)
    }

    override fun onCleared() {
        password = ""
    }

    companion object {
        const val MIN_PASSWORD = 8
    }
}
