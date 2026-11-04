@file:OptIn(ExperimentalMaterial3Api::class)

package app.photovault.android.ui.backup

import android.content.Intent
import android.net.Uri
import android.provider.Settings
import android.text.format.DateUtils
import android.text.format.Formatter
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.selection.toggleable
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Checkbox
import androidx.compose.material3.CheckboxDefaults
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Switch
import androidx.compose.material3.SwitchDefaults
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBar
import androidx.compose.material3.TopAppBarDefaults
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.res.pluralStringResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.LifecycleResumeEffect
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import app.photovault.android.R
import app.photovault.android.backup.BackupOutcome
import app.photovault.android.backup.BackupPhase
import app.photovault.android.backup.MediaAccess
import app.photovault.android.backup.RunState
import app.photovault.android.backup.backupPermissions
import app.photovault.android.ui.components.LinkButton
import app.photovault.android.ui.components.PrimaryButton
import app.photovault.android.ui.components.SecondaryButton
import app.photovault.android.ui.components.SectionLabel
import app.photovault.android.ui.theme.EncoreRadius
import app.photovault.android.ui.theme.Spacing
import app.photovault.android.ui.theme.palette

/** Camera backup: on/off, folders, Wi-Fi and charging options, and what the backup is doing. */
@Composable
fun BackupScreen(vm: BackupViewModel, onBack: () -> Unit) {
    val state by vm.state.collectAsStateWithLifecycle()
    val p = MaterialTheme.palette
    val context = LocalContext.current
    var enableAfterGrant by rememberSaveable { mutableStateOf(false) }
    val permissions = rememberLauncherForActivityResult(ActivityResultContracts.RequestMultiplePermissions()) {
        vm.onPermissionsResult(enableAfterGrant)
        enableAfterGrant = false
    }
    val askAccess = { enable: Boolean ->
        enableAfterGrant = enable
        permissions.launch(backupPermissions())
    }
    val openAppSettings = {
        context.startActivity(
            Intent(Settings.ACTION_APPLICATION_DETAILS_SETTINGS, Uri.fromParts("package", context.packageName, null))
                .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK),
        )
    }
    // Permissions can change in the system settings while the screen is away.
    LifecycleResumeEffect(vm) {
        vm.refresh()
        onPauseOrDispose { }
    }

    Scaffold(
        containerColor = p.bg,
        topBar = {
            TopAppBar(
                colors = TopAppBarDefaults.topAppBarColors(containerColor = p.bg, titleContentColor = p.text),
                navigationIcon = {
                    IconButton(onClick = onBack) {
                        Icon(painterResource(R.drawable.ic_arrow_back), contentDescription = stringResource(R.string.back), tint = p.text)
                    }
                },
                title = { Text(stringResource(R.string.backup_title), style = MaterialTheme.typography.titleLarge) },
            )
        },
    ) { padding ->
        val s = state.settings
        LazyColumn(
            modifier = Modifier.fillMaxSize().testTag("backup_list"),
            contentPadding = PaddingValues(
                start = Spacing.xl,
                end = Spacing.xl,
                top = padding.calculateTopPadding() + Spacing.md,
                bottom = padding.calculateBottomPadding() + Spacing.section,
            ),
            verticalArrangement = Arrangement.spacedBy(Spacing.md),
        ) {
            item(key = "status", contentType = "status") {
                StatusCard(state, onBackUpNow = vm::backUpNow)
            }
            if (state.loaded && state.access != MediaAccess.Full) {
                item(key = "access", contentType = "access") {
                    AccessCard(state.access, onAllow = { askAccess(false) }, onSettings = openAppSettings)
                }
            }
            item(key = "enabled", contentType = "switch") {
                SwitchRow(
                    title = stringResource(R.string.backup_enable),
                    subtitle = stringResource(R.string.backup_enable_detail),
                    checked = s.enabled,
                    enabled = state.signedIn,
                    onChange = { on ->
                        if (on && state.access == MediaAccess.None) askAccess(true) else vm.setEnabled(on)
                    },
                )
            }
            item(key = "folders_label", contentType = "label") {
                SectionLabel(stringResource(R.string.backup_folders))
            }
            if (state.loaded && state.folders.isEmpty()) {
                item(key = "folders_empty", contentType = "note") {
                    Note(stringResource(if (state.access == MediaAccess.None) R.string.backup_folders_no_access else R.string.backup_folders_none))
                }
            }
            items(state.folders, key = { "f:" + it.id }, contentType = { "folder" }) { f ->
                FolderRowView(f, onToggle = { vm.toggleFolder(f.id) })
            }
            item(key = "options_label", contentType = "label") {
                SectionLabel(stringResource(R.string.backup_options))
            }
            item(key = "wifi", contentType = "switch") {
                SwitchRow(stringResource(R.string.backup_wifi_only), stringResource(R.string.backup_wifi_only_detail), s.wifiOnly, onChange = vm::setWifiOnly)
            }
            item(key = "charging", contentType = "switch") {
                SwitchRow(stringResource(R.string.backup_charging_only), stringResource(R.string.backup_charging_only_detail), s.chargingOnly, onChange = vm::setChargingOnly)
            }
            item(key = "videos", contentType = "switch") {
                SwitchRow(stringResource(R.string.backup_videos), null, s.includeVideos, onChange = vm::setIncludeVideos)
            }
            item(key = "note", contentType = "note") {
                Note(stringResource(R.string.backup_note), Modifier.padding(top = Spacing.lg))
            }
        }
    }
}

@Composable
private fun Panel(modifier: Modifier = Modifier, content: @Composable () -> Unit) {
    val p = MaterialTheme.palette
    val shape = RoundedCornerShape(EncoreRadius.card)
    Column(
        modifier = modifier
            .fillMaxWidth()
            .clip(shape)
            .background(p.panel, shape)
            .border(1.dp, p.border, shape)
            .padding(Spacing.xl),
        verticalArrangement = Arrangement.spacedBy(Spacing.md),
    ) { content() }
}

@Composable
private fun StatusCard(state: BackupUiState, onBackUpNow: () -> Unit) {
    val p = MaterialTheme.palette
    val context = LocalContext.current
    val s = state.settings
    val q = state.queue
    val left = (q?.pending ?: 0) + (q?.uploading ?: 0)
    val phase = state.phase
    val last = state.last
    val (title, detail) = when {
        !state.loaded -> stringResource(R.string.backup_status_loading) to null
        !state.signedIn -> stringResource(R.string.backup_status_signed_out) to null
        !s.enabled -> stringResource(R.string.backup_status_off) to stringResource(R.string.backup_status_off_detail)
        state.access == MediaAccess.None -> stringResource(R.string.backup_status_no_access) to null
        phase is BackupPhase.Scanning -> stringResource(R.string.backup_status_scanning) to null
        phase is BackupPhase.Uploading || state.runState == RunState.Running && left > 0 -> {
            val st = (phase as? BackupPhase.Uploading)?.status ?: q
            pluralStringResource(R.plurals.backup_status_uploading, left, left) to st?.let {
                stringResource(
                    R.string.backup_status_bytes,
                    Formatter.formatShortFileSize(context, it.bytesDone),
                    Formatter.formatShortFileSize(context, it.bytesTotal),
                )
            }
        }
        left > 0 -> pluralStringResource(R.plurals.backup_status_waiting, left, left) to stringResource(
            when {
                s.wifiOnly && s.chargingOnly -> R.string.backup_waiting_wifi_charging
                s.wifiOnly -> R.string.backup_waiting_wifi
                s.chargingOnly -> R.string.backup_waiting_charging
                else -> R.string.backup_waiting_network
            },
        )
        last?.outcome == BackupOutcome.Error -> stringResource(R.string.backup_status_error) to last.message
        last?.outcome == BackupOutcome.NoPermission -> stringResource(R.string.backup_status_no_access) to null
        last != null && last.atMs > 0 -> stringResource(R.string.backup_status_done) to stringResource(
            R.string.backup_status_last,
            DateUtils.getRelativeTimeSpanString(last.atMs).toString(),
        )
        else -> stringResource(R.string.backup_status_pending) to null
    }
    Panel {
        Text(title, style = MaterialTheme.typography.titleMedium, color = p.text, modifier = Modifier.testTag("backup_status"))
        if (detail != null) Text(detail, style = MaterialTheme.typography.bodySmall, color = p.muted)
        val uploading = (phase as? BackupPhase.Uploading)?.status
        if (uploading != null && uploading.bytesTotal > 0) {
            LinearProgressIndicator(
                progress = { uploading.bytesDone.coerceAtMost(uploading.bytesTotal).toFloat() / uploading.bytesTotal },
                modifier = Modifier.fillMaxWidth(),
                color = p.accent2,
                trackColor = p.panel2,
                drawStopIndicator = {},
            )
        }
        val failed = q?.failed ?: 0
        if (failed > 0) {
            Text(pluralStringResource(R.plurals.backup_failed, failed, failed), style = MaterialTheme.typography.bodySmall, color = p.warn)
        }
        val skipped = last?.skipped ?: 0
        if (skipped > 0) {
            Text(pluralStringResource(R.plurals.backup_skipped, skipped, skipped), style = MaterialTheme.typography.bodySmall, color = p.muted)
        }
        if (state.signedIn && s.enabled && state.access != MediaAccess.None) {
            PrimaryButton(
                text = stringResource(R.string.backup_now),
                onClick = onBackUpNow,
                modifier = Modifier.fillMaxWidth().padding(top = Spacing.xs),
                enabled = phase == BackupPhase.Idle && state.runState != RunState.Running,
            )
        }
    }
}

@Composable
private fun AccessCard(access: MediaAccess, onAllow: () -> Unit, onSettings: () -> Unit) {
    val p = MaterialTheme.palette
    Panel {
        Text(
            stringResource(if (access == MediaAccess.Partial) R.string.backup_access_partial else R.string.backup_access_needed),
            style = MaterialTheme.typography.bodyMedium,
            color = p.text,
        )
        SecondaryButton(stringResource(R.string.backup_access_allow), onAllow, Modifier.fillMaxWidth())
        LinkButton(stringResource(R.string.backup_access_settings), onSettings)
    }
}

@Composable
private fun SwitchRow(title: String, subtitle: String?, checked: Boolean, enabled: Boolean = true, onChange: (Boolean) -> Unit) {
    val p = MaterialTheme.palette
    val shape = RoundedCornerShape(EncoreRadius.tile)
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .clip(shape)
            .background(p.panel, shape)
            .border(1.dp, p.border, shape)
            .toggleable(value = checked, enabled = enabled, role = Role.Switch, onValueChange = onChange)
            .padding(horizontal = Spacing.lg, vertical = Spacing.base),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(Spacing.lg),
    ) {
        Column(Modifier.weight(1f)) {
            Text(title, style = MaterialTheme.typography.titleMedium, color = if (enabled) p.text else p.faint)
            if (subtitle != null) Text(subtitle, style = MaterialTheme.typography.bodySmall, color = p.muted)
        }
        Switch(
            checked = checked,
            onCheckedChange = null,
            enabled = enabled,
            colors = SwitchDefaults.colors(
                checkedThumbColor = p.onAccent,
                checkedTrackColor = p.accent2,
                uncheckedThumbColor = p.muted,
                uncheckedTrackColor = p.panel2,
                uncheckedBorderColor = p.border,
            ),
        )
    }
}

@Composable
private fun FolderRowView(f: FolderRow, onToggle: () -> Unit) {
    val p = MaterialTheme.palette
    val shape = RoundedCornerShape(EncoreRadius.tile)
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .clip(shape)
            .background(p.panel, shape)
            .border(1.dp, p.border, shape)
            .clickable(role = Role.Checkbox, onClick = onToggle)
            .padding(horizontal = Spacing.lg, vertical = Spacing.sm),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(Spacing.md),
    ) {
        Column(Modifier.weight(1f)) {
            Text(f.name, style = MaterialTheme.typography.titleMedium, color = p.text)
            Text(pluralStringResource(R.plurals.backup_folder_count, f.count, f.count), style = MaterialTheme.typography.bodySmall, color = p.muted)
        }
        Checkbox(
            checked = f.selected,
            onCheckedChange = null,
            colors = CheckboxDefaults.colors(checkedColor = p.accent2, uncheckedColor = p.muted, checkmarkColor = p.onAccent),
        )
    }
}

@Composable
private fun Note(text: String, modifier: Modifier = Modifier) {
    Text(text, modifier = modifier, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.palette.muted)
}
