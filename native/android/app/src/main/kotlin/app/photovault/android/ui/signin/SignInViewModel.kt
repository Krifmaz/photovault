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
)

/** Where the sign-in goes next (LoginStep -> screen). */
sealed interface SignInEvent {
    data object EmailCode : SignInEvent
    data class Totp(val sessionId: String) : SignInEvent
    data class Passkey(val sessionId: String, val url: String) : SignInEvent
    data object RecoveryKey : SignInEvent
    data object Done : SignInEvent
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

    val state: StateFlow<SignInUiState> = _state.asStateFlow()
    val eventFlow: Flow<SignInEvent> = events.receiveAsFlow()

    init {
        viewModelScope.launch {
            val label = withContext(io) {
                val active = core.activeBaseUrl()
                val server = core.savedServers().firstOrNull { it.publicUrl == active || it.homeUrl == active }
                server?.name?.takeIf { it.isNotBlank() } ?: server?.publicUrl ?: active.orEmpty()
            }
            _state.update { it.copy(serverLabel = label) }
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

    fun verifyEmailCode(code: String) {
        if (code.isBlank()) return problem(SignInProblem.MissingFields)
        runStep { core.verifyEmailOtt(_state.value.email, code.trim(), password) }
    }

    fun verifyTotp(sessionId: String, code: String) {
        if (code.isBlank()) return problem(SignInProblem.MissingFields)
        runStep { core.verifyTotp(sessionId, code.trim(), password) }
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
    private fun runStep(step: suspend () -> LoginStep) = launchBusy {
        when (val next = step()) {
            is LoginStep.Done -> finish()
            LoginStep.EmailOtt -> {
                core.sendEmailOtt(_state.value.email)
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
