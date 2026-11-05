package app.photovault.android.ui.connect

import android.text.format.DateUtils
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawing
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import app.photovault.android.R
import app.photovault.android.data.RecentServer
import app.photovault.android.data.ServerInfo
import app.photovault.android.data.displayAddress
import app.photovault.android.ui.components.PanelRow
import app.photovault.android.ui.components.PrimaryButton
import app.photovault.android.ui.components.ProblemText
import app.photovault.android.ui.components.PvTextField
import app.photovault.android.ui.components.ScreenHeader
import app.photovault.android.ui.components.SecondaryButton
import app.photovault.android.ui.components.SectionLabel
import app.photovault.android.ui.theme.EncoreRadius
import app.photovault.android.ui.theme.Spacing
import app.photovault.android.ui.theme.palette
import kotlinx.coroutines.flow.Flow

/** "Connect to your server": address, LAN servers, servers reached over Tailscale, QR, recent servers. */
@Composable
fun ConnectScreen(
    vm: ConnectViewModel,
    qrPayloads: Flow<String?>,
    onQrConsumed: () -> Unit,
    onScanQr: () -> Unit,
    onSignIn: () -> Unit,
    onTimeline: () -> Unit,
) {
    val signIn by rememberUpdatedState(onSignIn)
    val timeline by rememberUpdatedState(onTimeline)
    LaunchedEffect(vm) {
        vm.eventFlow.collect { e ->
            when (e) {
                ConnectEvent.SignIn -> signIn()
                ConnectEvent.Timeline -> timeline()
            }
        }
    }
    LaunchedEffect(qrPayloads) {
        qrPayloads.collect { payload ->
            if (payload != null) {
                onQrConsumed()
                vm.onQrScanned(payload)
            }
        }
    }

    LazyColumn(
        modifier = Modifier
            .fillMaxSize()
            .background(MaterialTheme.palette.bg)
            .windowInsetsPadding(WindowInsets.safeDrawing)
            .imePadding(),
        contentPadding = PaddingValues(horizontal = Spacing.xxl, vertical = Spacing.section),
        verticalArrangement = Arrangement.spacedBy(Spacing.md),
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        item(key = "header", contentType = "header") {
            ScreenHeader(
                title = stringResource(R.string.connect_title),
                subtitle = stringResource(R.string.connect_subtitle),
                modifier = Modifier.widthIn(max = 560.dp),
            )
        }
        item(key = "form", contentType = "form") {
            AddressForm(vm, onScanQr, Modifier.widthIn(max = 560.dp).padding(top = Spacing.xl))
        }
        lanSection(vm)
        tailscaleSection(vm)
        savedSection(vm)
    }
}

@Composable
private fun AddressForm(vm: ConnectViewModel, onScanQr: () -> Unit, modifier: Modifier = Modifier) {
    val state by vm.state.collectAsStateWithLifecycle()
    val p = MaterialTheme.palette
    Column(modifier.fillMaxWidth(), verticalArrangement = Arrangement.spacedBy(Spacing.lg)) {
        PvTextField(
            value = state.address,
            onValueChange = vm::onAddressChange,
            label = stringResource(R.string.connect_address),
            placeholder = stringResource(R.string.connect_address_hint),
            keyboardType = KeyboardType.Uri,
            imeAction = ImeAction.Go,
            onIme = vm::check,
            isError = state.problem != null,
            trailing = {
                IconButton(onClick = onScanQr) {
                    Icon(painterResource(R.drawable.ic_qr), contentDescription = stringResource(R.string.connect_scan_qr), tint = p.accent2)
                }
            },
        )
        state.problem?.let { ProblemText(problemText(it)) }
        val info = state.info
        if (info == null) {
            PrimaryButton(
                text = stringResource(R.string.connect_check),
                onClick = vm::check,
                loading = state.checking,
                modifier = Modifier.fillMaxWidth(),
            )
        } else {
            ServerCard(info)
            PrimaryButton(
                text = stringResource(R.string.connect_continue),
                onClick = vm::connect,
                loading = state.connecting,
                modifier = Modifier.fillMaxWidth(),
            )
        }
        SecondaryButton(
            text = stringResource(R.string.connect_scan_qr),
            onClick = onScanQr,
            modifier = Modifier.fillMaxWidth(),
        )
    }
}

@Composable
private fun ServerCard(info: ServerInfo) {
    val p = MaterialTheme.palette
    val shape = RoundedCornerShape(EncoreRadius.card)
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .clip(shape)
            .background(p.panel, shape)
            .border(1.dp, p.ok.copy(alpha = 0.5f), shape)
            .padding(Spacing.xl),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(Spacing.lg),
    ) {
        Icon(painterResource(R.drawable.ic_check_circle), contentDescription = null, tint = p.ok, modifier = Modifier.size(28.dp))
        Column(Modifier.weight(1f)) {
            if (info.isPhotovault) {
                Text(info.name.ifBlank { stringResource(R.string.app_name_short) }, style = MaterialTheme.typography.titleLarge, color = p.text)
                Text(stringResource(R.string.connect_version, info.version), style = MaterialTheme.typography.bodySmall, color = p.muted)
            } else {
                Text(stringResource(R.string.connect_compatible), style = MaterialTheme.typography.titleLarge, color = p.text)
                Text(stringResource(R.string.connect_compatible_detail), style = MaterialTheme.typography.bodySmall, color = p.muted)
            }
            Text(displayAddress(info.publicUrl), style = MaterialTheme.typography.bodySmall, color = p.faint)
        }
    }
}

@Composable
private fun problemText(problem: ConnectProblem): String = when (problem) {
    is ConnectProblem.Unreachable -> stringResource(R.string.connect_unreachable, problem.address)
    is ConnectProblem.NotPhotovault -> stringResource(R.string.connect_not_photovault, problem.address)
    ConnectProblem.EmptyAddress -> stringResource(R.string.connect_empty)
    ConnectProblem.BadQr -> stringResource(R.string.connect_bad_qr)
    is ConnectProblem.Other -> stringResource(R.string.connect_other, problem.message)
}

private fun androidx.compose.foundation.lazy.LazyListScope.lanSection(vm: ConnectViewModel) {
    item(key = "lan-label", contentType = "label") {
        Row(Modifier.fillMaxWidth().widthIn(max = 560.dp), verticalAlignment = Alignment.CenterVertically) {
            SectionLabel(stringResource(R.string.connect_on_network), Modifier.weight(1f))
            CircularProgressIndicator(
                modifier = Modifier.padding(top = Spacing.xl).size(14.dp),
                strokeWidth = 2.dp,
                color = MaterialTheme.palette.faint,
            )
        }
    }
    item(key = "lan-list", contentType = "lan") {
        LanList(vm)
    }
}

@Composable
private fun LanList(vm: ConnectViewModel) {
    val servers by vm.lan.collectAsStateWithLifecycle()
    Column(Modifier.fillMaxWidth().widthIn(max = 560.dp), verticalArrangement = Arrangement.spacedBy(Spacing.md)) {
        if (servers.isEmpty()) {
            Text(
                stringResource(R.string.connect_on_network_none),
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.palette.muted,
            )
        }
        for (server in servers) {
            PanelRow(
                title = server.name,
                subtitle = listOfNotNull(displayAddress(server.address), server.version?.let { "v$it" }).joinToString(" · "),
                onClick = { vm.useDiscovered(server) },
                icon = R.drawable.ic_wifi,
            )
        }
    }
}

private fun androidx.compose.foundation.lazy.LazyListScope.tailscaleSection(vm: ConnectViewModel) {
    item(key = "tailscale", contentType = "tailscale") {
        TailscaleList(vm)
    }
}

/** Shown while Tailscale is on: recent servers that answered directly (mDNS does not cross the VPN). */
@Composable
private fun TailscaleList(vm: ConnectViewModel) {
    val scan by vm.tailscale.collectAsStateWithLifecycle()
    val state by vm.state.collectAsStateWithLifecycle()
    if (!scan.active) return
    val p = MaterialTheme.palette
    Column(Modifier.fillMaxWidth().widthIn(max = 560.dp), verticalArrangement = Arrangement.spacedBy(Spacing.md)) {
        Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
            SectionLabel(stringResource(R.string.connect_tailscale), Modifier.weight(1f))
            if (scan.searching) {
                CircularProgressIndicator(
                    modifier = Modifier.padding(top = Spacing.xl).size(14.dp),
                    strokeWidth = 2.dp,
                    color = p.faint,
                )
            }
        }
        val note = when {
            scan.searching -> stringResource(R.string.connect_tailscale_searching)
            scan.found.isNotEmpty() -> null
            state.recent.isEmpty() -> stringResource(R.string.connect_tailscale_no_recent)
            else -> stringResource(R.string.connect_tailscale_none)
        }
        if (note != null) Text(note, style = MaterialTheme.typography.bodySmall, color = p.muted)
        for (reach in scan.found) {
            val shown = displayAddress(reach.address)
            PanelRow(
                title = reach.info.name.ifBlank { displayAddress(reach.info.publicUrl) },
                subtitle = if (reach.direct) stringResource(R.string.connect_tailscale_reached, shown)
                else stringResource(R.string.connect_tailscale_reached_public, shown),
                onClick = { vm.useTailscale(reach) },
                icon = R.drawable.ic_lock,
            )
        }
    }
}

/** Opt-in for a plain-http home address (see ConnectViewModel.setHomeAllowed). */
@Composable
private fun HomeAddressRow(title: String, warning: String, allowed: Boolean, onChange: (Boolean) -> Unit) {
    val p = MaterialTheme.palette
    Row(
        Modifier.fillMaxWidth().padding(horizontal = Spacing.md),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(Spacing.md),
    ) {
        Column(Modifier.weight(1f)) {
            Text(title, color = p.text, style = MaterialTheme.typography.bodyMedium)
            Text(warning, color = p.muted, style = MaterialTheme.typography.bodySmall)
        }
        Switch(checked = allowed, onCheckedChange = onChange)
    }
}

private fun androidx.compose.foundation.lazy.LazyListScope.savedSection(vm: ConnectViewModel) {
    item(key = "saved", contentType = "saved") {
        SavedList(vm)
    }
}

@Composable
private fun SavedList(vm: ConnectViewModel) {
    val state by vm.state.collectAsStateWithLifecycle()
    if (state.recent.isEmpty()) return
    val p = MaterialTheme.palette
    Column(Modifier.fillMaxWidth().widthIn(max = 560.dp), verticalArrangement = Arrangement.spacedBy(Spacing.md)) {
        SectionLabel(stringResource(R.string.connect_recent))
        for (recent in state.recent) {
            val server = recent.info
            val active = state.activeUrl != null &&
                (state.activeUrl == server.publicUrl || state.activeUrl == server.homeUrl || state.activeUrl == recent.lastAddress)
            PanelRow(
                title = server.name.ifBlank { displayAddress(server.publicUrl) },
                subtitle = recentSubtitle(recent, active),
                onClick = { vm.selectSaved(server) },
                icon = R.drawable.ic_server,
                trailing = {
                    IconButton(onClick = { vm.removeSaved(server) }) {
                        Icon(
                            painterResource(R.drawable.ic_delete),
                            contentDescription = stringResource(R.string.connect_saved_remove, server.name.ifBlank { server.publicUrl }),
                            tint = p.muted,
                        )
                    }
                },
            )
            val allowed = server.publicUrl in state.homeAllowed
            val home = server.homeUrl?.takeIf { it.startsWith("http://", ignoreCase = true) }
            val direct = recent.lastAddress
                ?.takeIf { it.startsWith("http://", ignoreCase = true) }
                ?.takeIf { it.trimEnd('/') != server.publicUrl.trimEnd('/') && it.trimEnd('/') != server.homeUrl?.trimEnd('/') }
            // One approval covers both plain-http addresses (contract v1.3); name the home one if there is one.
            if (home != null) {
                HomeAddressRow(
                    title = stringResource(R.string.connect_home_title, displayAddress(home)),
                    warning = stringResource(R.string.connect_home_warning),
                    allowed = allowed,
                    onChange = { vm.setHomeAllowed(server, it) },
                )
            } else if (direct != null) {
                HomeAddressRow(
                    title = stringResource(R.string.connect_direct_title, displayAddress(direct)),
                    warning = stringResource(R.string.connect_direct_warning),
                    allowed = allowed,
                    onChange = { vm.setHomeAllowed(server, it) },
                )
            }
        }
        Spacer(Modifier.height(Spacing.xl))
    }
}

/** "photos.example.com · current", or "· last connected 2 days ago". */
@Composable
private fun recentSubtitle(recent: RecentServer, active: Boolean): String {
    val address = displayAddress(recent.info.publicUrl)
    return when {
        active -> stringResource(R.string.connect_saved_active, address)
        recent.lastConnectedUs > 0 -> stringResource(
            R.string.connect_recent_last,
            address,
            DateUtils.getRelativeTimeSpanString(
                recent.lastConnectedUs / 1_000L,
                System.currentTimeMillis(),
                DateUtils.MINUTE_IN_MILLIS,
            ).toString(),
        )
        else -> address
    }
}
