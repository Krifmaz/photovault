package app.photovault.android.ui.connect

import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import app.photovault.android.data.CoreError
import app.photovault.android.data.CoreRepository
import app.photovault.android.data.DiscoveredServer
import app.photovault.android.data.LanDiscovery
import app.photovault.android.data.ServerInfo
import app.photovault.android.data.displayAddress
import kotlinx.coroutines.CoroutineDispatcher
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.SharingStarted
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.catch
import kotlinx.coroutines.flow.receiveAsFlow
import kotlinx.coroutines.flow.stateIn
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

/** Why the address did not work; the screen turns it into a sentence. */
sealed interface ConnectProblem {
    data class Unreachable(val address: String) : ConnectProblem
    data class NotPhotovault(val address: String) : ConnectProblem
    data object EmptyAddress : ConnectProblem
    data object BadQr : ConnectProblem
    data class Other(val message: String) : ConnectProblem
}

data class ConnectUiState(
    val address: String = "",
    val checking: Boolean = false,
    val connecting: Boolean = false,
    /** The server [address] answered with, once checked. */
    val info: ServerInfo? = null,
    val problem: ConnectProblem? = null,
    val saved: List<ServerInfo> = emptyList(),
    val activeUrl: String? = null,
)

sealed interface ConnectEvent {
    data object SignIn : ConnectEvent
    data object Timeline : ConnectEvent
}

class ConnectViewModel(
    private val core: CoreRepository,
    discovery: LanDiscovery,
    defaultServer: String,
    private val io: CoroutineDispatcher = Dispatchers.IO,
) : ViewModel() {

    private val _state = MutableStateFlow(ConnectUiState(address = defaultServer))
    private val events = Channel<ConnectEvent>(Channel.BUFFERED)
    private var checkJob: Job? = null

    val state: StateFlow<ConnectUiState> = _state.asStateFlow()
    val eventFlow: Flow<ConnectEvent> = events.receiveAsFlow()

    /** Servers announced on this network, while the screen is visible. */
    val lan: StateFlow<List<DiscoveredServer>> = discovery.servers()
        .catch { emit(emptyList()) }
        .stateIn(viewModelScope, SharingStarted.WhileSubscribed(5_000), emptyList())

    init {
        reloadSaved()
    }

    private fun reloadSaved() {
        viewModelScope.launch {
            val (saved, active) = withContext(io) { core.savedServers() to core.activeBaseUrl() }
            _state.update { it.copy(saved = saved, activeUrl = active) }
        }
    }

    fun onAddressChange(text: String) {
        checkJob?.cancel()
        _state.update { it.copy(address = text, info = null, problem = null, checking = false) }
    }

    /** fetch_server_info for the typed address: shows the server's name and version, or why not. */
    fun check() {
        val address = _state.value.address.trim()
        if (address.isEmpty()) {
            _state.update { it.copy(problem = ConnectProblem.EmptyAddress) }
            return
        }
        checkJob?.cancel()
        checkJob = viewModelScope.launch {
            _state.update { it.copy(checking = true, info = null, problem = null) }
            try {
                val info = core.fetchServerInfo(address)
                _state.update { it.copy(checking = false, info = info) }
            } catch (e: CoreError) {
                _state.update { it.copy(checking = false, problem = e.toProblem(address)) }
            }
        }
    }

    /** Saves the checked server, makes it the active one and moves on to sign-in. */
    fun connect() {
        val address = _state.value.address.trim()
        if (address.isEmpty() || _state.value.connecting) return
        viewModelScope.launch {
            _state.update { it.copy(connecting = true, problem = null) }
            try {
                val info = core.addServer(address)
                withContext(io) { core.selectServer(info.publicUrl) }
                _state.update { it.copy(connecting = false, info = info) }
                reloadSaved()
                events.send(nextStep(info.publicUrl))
            } catch (e: CoreError) {
                _state.update { it.copy(connecting = false, problem = e.toProblem(address)) }
            }
        }
    }

    /** A server tapped in the LAN list: fill in its address and check it. */
    fun useDiscovered(server: DiscoveredServer) {
        onAddressChange(server.address)
        check()
    }

    /** A scanned QR payload: core's parse_connect_qr, then check the address it names. */
    fun onQrScanned(payload: String) {
        viewModelScope.launch {
            val address = withContext(io) { core.parseConnectQr(payload) }
            if (address == null) {
                _state.update { it.copy(problem = ConnectProblem.BadQr) }
            } else {
                onAddressChange(address)
                check()
            }
        }
    }

    fun selectSaved(server: ServerInfo) {
        viewModelScope.launch {
            try {
                withContext(io) { core.selectServer(server.publicUrl) }
                reloadSaved()
                events.send(nextStep(server.publicUrl))
            } catch (e: CoreError) {
                _state.update { it.copy(problem = e.toProblem(server.publicUrl)) }
            }
        }
    }

    fun removeSaved(server: ServerInfo) {
        viewModelScope.launch {
            try {
                withContext(io) { core.removeServer(server.publicUrl) }
            } catch (e: CoreError) {
                _state.update { it.copy(problem = e.toProblem(server.publicUrl)) }
            }
            reloadSaved()
        }
    }

    private suspend fun nextStep(publicUrl: String): ConnectEvent {
        val account = withContext(io) { core.currentAccount() }
        return if (account != null && account.serverUrl.trimEnd('/') == publicUrl.trimEnd('/')) ConnectEvent.Timeline
        else ConnectEvent.SignIn
    }

    private fun CoreError.toProblem(address: String): ConnectProblem = when (this) {
        is CoreError.Unreachable -> ConnectProblem.Unreachable(this.address.ifBlank { displayAddress(address) })
        is CoreError.NotPhotovault -> ConnectProblem.NotPhotovault(this.address.ifBlank { displayAddress(address) })
        else -> ConnectProblem.Other(message ?: javaClass.simpleName)
    }
}
