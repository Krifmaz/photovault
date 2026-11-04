package app.photovault.android.ui.signup

import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import app.photovault.android.data.CoreError
import app.photovault.android.data.CoreRepository
import app.photovault.android.data.SignupRepository
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

sealed interface SignupProblem {
    data object MissingFields : SignupProblem
    data object InvalidEmail : SignupProblem
    data object PasswordsDiffer : SignupProblem
    data object PasswordTooShort : SignupProblem
    data object PasswordTooWeak : SignupProblem
    data object IncorrectCode : SignupProblem
    data object AlreadyRegistered : SignupProblem
    data object SignupsClosed : SignupProblem
    data object NotSaved : SignupProblem
    data class Unreachable(val address: String) : SignupProblem
    data class Other(val message: String) : SignupProblem
}

data class SignupUiState(
    val serverLabel: String = "",
    /** From the saved server info; null until read. Core asks the server again before sending a code. */
    val signupsAllowed: Boolean? = null,
    val email: String = "",
    val busy: Boolean = false,
    /** The code was checked and the keys are being made (several seconds of Argon2id). */
    val makingKeys: Boolean = false,
    val problem: SignupProblem? = null,
    /** The recovery key, once made (memory only, never saved state). */
    val recoveryWords: List<String> = emptyList(),
    val savedConfirmed: Boolean = false,
)

/** Where the sign-up goes next. */
sealed interface SignupEvent {
    data object Password : SignupEvent
    data object Code : SignupEvent
    data object RecoveryKey : SignupEvent
    data object Done : SignupEvent
    /** The password is gone (process restarted mid sign-up): start over at the email screen. */
    data object Restart : SignupEvent
}

/**
 * Create account, shared by the email, password, email code and recovery key screens (scoped to the sign-up
 * graph). The password stays in this view model only until the keys are made.
 */
class SignupViewModel(
    private val core: CoreRepository,
    private val signup: SignupRepository,
    private val io: CoroutineDispatcher = Dispatchers.IO,
) : ViewModel() {

    private val _state = MutableStateFlow(SignupUiState())
    private val events = Channel<SignupEvent>(Channel.BUFFERED)
    private var password: String = ""

    val state: StateFlow<SignupUiState> = _state.asStateFlow()
    val eventFlow: Flow<SignupEvent> = events.receiveAsFlow()

    init {
        viewModelScope.launch {
            val (label, allowed) = withContext(io) {
                val active = core.activeBaseUrl()
                val server = core.savedServers().firstOrNull { it.publicUrl == active || it.homeUrl == active }
                (server?.name?.takeIf { it.isNotBlank() } ?: server?.publicUrl ?: active.orEmpty()) to server?.signupsAllowed
            }
            _state.update { it.copy(serverLabel = label, signupsAllowed = allowed) }
        }
    }

    fun onEmailChange(email: String) = _state.update { it.copy(email = email, problem = null) }

    fun clearProblem() = _state.update { it.copy(problem = null) }

    /** Email screen: a plausible address, then the password screen. */
    fun submitEmail() {
        val e = _state.value.email.trim()
        when {
            e.isEmpty() -> return problem(SignupProblem.MissingFields)
            !looksLikeEmail(e) -> return problem(SignupProblem.InvalidEmail)
        }
        _state.update { it.copy(email = e, problem = null) }
        viewModelScope.launch { events.send(SignupEvent.Password) }
    }

    /** Password screen: checks the password, then emails the code. */
    fun submitPassword(password: String, confirm: String) {
        when {
            password.isEmpty() || confirm.isEmpty() -> return problem(SignupProblem.MissingFields)
            password != confirm -> return problem(SignupProblem.PasswordsDiffer)
            password.length < MIN_PASSWORD -> return problem(SignupProblem.PasswordTooShort)
            PasswordStrengthEstimator.level(password) == PasswordStrength.Weak -> return problem(SignupProblem.PasswordTooWeak)
        }
        this.password = password
        launchBusy {
            signup.sendCode(_state.value.email)
            events.send(SignupEvent.Code)
        }
    }

    fun resendCode() = launchBusy { signup.sendCode(_state.value.email) }

    /** Code screen: checks the code, then makes the keys and shows the recovery key. */
    fun verifyCode(code: String) {
        if (code.isBlank()) return problem(SignupProblem.MissingFields)
        if (password.isEmpty()) {
            viewModelScope.launch { events.send(SignupEvent.Restart) }
            return
        }
        launchBusy {
            signup.verifyCode(_state.value.email, code.trim())
            _state.update { it.copy(makingKeys = true) }
            try {
                val words = signup.prepare(password)
                password = ""
                _state.update { it.copy(recoveryWords = words, savedConfirmed = false) }
                events.send(SignupEvent.RecoveryKey)
            } finally {
                _state.update { it.copy(makingKeys = false) }
            }
        }
    }

    fun setSaved(saved: Boolean) = _state.update { it.copy(savedConfirmed = saved, problem = null) }

    /** Recovery key screen: only after "I saved my recovery key". */
    fun finish() {
        if (!_state.value.savedConfirmed) return problem(SignupProblem.NotSaved)
        if (_state.value.recoveryWords.isEmpty()) {
            viewModelScope.launch { events.send(SignupEvent.Restart) }
            return
        }
        launchBusy {
            signup.finish()
            _state.update { it.copy(recoveryWords = emptyList()) }
            events.send(SignupEvent.Done)
        }
    }

    private fun problem(p: SignupProblem) = _state.update { it.copy(problem = p) }

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

    private fun CoreError.toProblem(): SignupProblem = when (this) {
        is CoreError.IncorrectCode -> SignupProblem.IncorrectCode
        is CoreError.Unreachable -> SignupProblem.Unreachable(address)
        is CoreError.Server -> when (status) {
            409 -> SignupProblem.AlreadyRegistered
            403 -> SignupProblem.SignupsClosed.also { _state.update { s -> s.copy(signupsAllowed = false) } }
            else -> SignupProblem.Other(message ?: "Server error $status")
        }
        else -> SignupProblem.Other(message ?: javaClass.simpleName)
    }

    override fun onCleared() {
        password = ""
    }

    companion object {
        const val MIN_PASSWORD = 8

        fun looksLikeEmail(e: String): Boolean {
            val at = e.indexOf('@')
            return at > 0 && at == e.lastIndexOf('@') && at < e.length - 1 && e.none { it.isWhitespace() }
        }
    }
}
