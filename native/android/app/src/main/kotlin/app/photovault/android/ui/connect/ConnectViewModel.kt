package app.photovault.android.ui.connect

import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import app.photovault.android.data.CoreError
import app.photovault.android.data.CoreRepository
import app.photovault.android.data.DiscoveredServer
import app.photovault.android.data.LanDiscovery
import app.photovault.android.data.RecentServer
import app.photovault.android.data.ServerInfo
import app.photovault.android.data.TailscaleStatus
import app.photovault.android.data.displayAddress
import kotlinx.coroutines.CoroutineDispatcher
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.Job
import kotlinx.coroutines.async
import kotlinx.coroutines.awaitAll
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.coroutineScope
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.SharingStarted
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.catch
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.flow.distinctUntilChanged
import kotlinx.coroutines.flow.map
import kotlinx.coroutines.flow.receiveAsFlow
import kotlinx.coroutines.flow.stateIn
import kotlinx.coroutines.flow.transformLatest
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import kotlinx.coroutines.withTimeoutOrNull

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
    /** Saved servers, most recently reached first ("Recent servers"). */
    val recent: List<RecentServer> = emptyList(),
    val activeUrl: String? = null,
    /** Saved servers (public URLs) whose plain-http home (or last direct) address the user allowed. */
    val homeAllowed: Set<String> = emptySet(),
) {
    /** The saved servers, in [recent] order. */
    val saved: List<ServerInfo> get() = recent.map { it.info }
}

/** A recent server that answered directly while Tailscale is on, at [address]. */
data class TailscaleReach(val server: RecentServer, val address: String, val info: ServerInfo) {
    /** False when only its public URL answered (that may not have gone through Tailscale). */
    val direct: Boolean get() = address.trimEnd('/') != server.info.publicUrl.trimEnd('/')
}

/** Recent servers tried directly while Tailscale is on (mDNS does not cross the VPN). */
data class TailscaleScan(
    val active: Boolean = false,
    val searching: Boolean = false,
    val found: List<TailscaleReach> = emptyList(),
)

sealed interface ConnectEvent {
    data object SignIn : ConnectEvent
    data object Timeline : ConnectEvent
}

class ConnectViewModel(
    private val core: CoreRepository,
    discovery: LanDiscovery,
    defaultServer: String,
    tailscaleStatus: TailscaleStatus = TailscaleStatus.Off,
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

    /**
     * While Tailscale is on: each recent server's last address that answered, then its home and public URL, tried
     * directly with [fetchServerInfo][CoreRepository.fetchServerInfo] (a short timeout each); the ones that answer
     * as that server.
     */
    @OptIn(ExperimentalCoroutinesApi::class)
    val tailscale: StateFlow<TailscaleScan> = tailscaleStatus.active()
        .catch { emit(false) }
        .distinctUntilChanged()
        .combine(_state.map { s -> s.recent.map { it.probeTargets() } }.distinctUntilChanged()) { on, _ -> on }
        .transformLatest { on ->
            if (!on) {
                emit(TailscaleScan())
                return@transformLatest
            }
            val recent = _state.value.recent
            emit(TailscaleScan(active = true, searching = recent.isNotEmpty()))
            val found = coroutineScope { recent.map { async { reach(it) } }.awaitAll().filterNotNull() }
            emit(TailscaleScan(active = true, searching = false, found = found))
        }
        .stateIn(viewModelScope, SharingStarted.WhileSubscribed(5_000), TailscaleScan())

    init {
        reloadSaved()
    }

    private fun reloadSaved() {
        viewModelScope.launch {
            val (recent, active, allowed) = withContext(io) {
                val recent = core.recentServers()
                Triple(recent, core.activeBaseUrl(), recent.filter { core.homeUrlAllowed(it.info.publicUrl) }.map { it.info.publicUrl }.toSet())
            }
            _state.update { it.copy(recent = recent, activeUrl = active, homeAllowed = allowed) }
        }
    }

    private fun RecentServer.probeTargets(): List<String> =
        listOfNotNull(lastAddress, info.homeUrl, info.publicUrl).map { it.trimEnd('/') }.filter { it.isNotBlank() }.distinct()

    private suspend fun reach(server: RecentServer): TailscaleReach? {
        val public = server.info.publicUrl.trimEnd('/')
        for (address in server.probeTargets()) {
            val info = withTimeoutOrNull(TAILSCALE_PROBE_MS) {
                try {
                    core.fetchServerInfo(address)
                } catch (e: CoreError) {
                    null
                }
            }
            if (info != null && info.publicUrl.trimEnd('/') == public) return TailscaleReach(server, address, info)
        }
        return null
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

    /**
     * A recent server tapped: it becomes the active one. Core then tries its last address that answered, then its
     * home URL, then its public URL (contract v1.3).
     */
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

    /**
     * Allows (or stops) using a server's plain-http home address. That address carries the sign-in and the
     * session token unencrypted, and on another network the same address can be someone else's machine,
     * so it is off until the user turns it on.
     */
    fun setHomeAllowed(server: ServerInfo, allowed: Boolean) {
        viewModelScope.launch {
            try {
                withContext(io) { core.setHomeUrlAllowed(server.publicUrl, allowed) }
            } catch (e: CoreError) {
                _state.update { it.copy(problem = e.toProblem(server.publicUrl)) }
            }
            reloadSaved()
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

    /** A server found over Tailscale tapped: the same as tapping it under "Recent servers". */
    fun useTailscale(reach: TailscaleReach) = selectSaved(reach.server.info)

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

    companion object {
        /** How long one address gets to answer over Tailscale. */
        const val TAILSCALE_PROBE_MS = 3_000L
    }
}
