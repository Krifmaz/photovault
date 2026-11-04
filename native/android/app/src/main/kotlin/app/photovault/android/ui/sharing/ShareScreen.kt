package app.photovault.android.ui.sharing

import android.content.ClipData
import android.content.ClipDescription
import android.content.ClipboardManager
import android.content.Context
import android.content.Intent
import android.os.Build
import android.os.PersistableBundle
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.FilterChip
import androidx.compose.material3.FilterChipDefaults
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.Switch
import androidx.compose.material3.SwitchDefaults
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import app.photovault.android.R
import app.photovault.android.data.albums.AlbumDetails
import app.photovault.android.data.albums.PublicLink
import app.photovault.android.data.albums.ShareRole
import app.photovault.android.data.albums.Sharee
import app.photovault.android.ui.albums.AlbumTopBar
import app.photovault.android.ui.albums.ConfirmDialog
import app.photovault.android.ui.albums.albumName
import app.photovault.android.ui.components.PasswordField
import app.photovault.android.ui.components.PrimaryButton
import app.photovault.android.ui.components.PvTextField
import app.photovault.android.ui.components.SecondaryButton
import app.photovault.android.ui.components.SectionLabel
import app.photovault.android.ui.theme.EncoreRadius
import app.photovault.android.ui.theme.Spacing
import app.photovault.android.ui.theme.palette
import kotlinx.coroutines.launch
import java.text.DateFormat
import java.util.Date

/** Share an album with people on the server, and manage its public link. */
@Composable
fun ShareScreen(vm: ShareViewModel, onBack: () -> Unit) {
    val state by vm.state.collectAsStateWithLifecycle()
    val p = MaterialTheme.palette
    val snackbar = remember { SnackbarHostState() }
    val context = LocalContext.current
    val copied = stringResource(R.string.share_link_copied)
    val scope = rememberCoroutineScope()

    LaunchedEffect(state.error) {
        state.error?.let {
            snackbar.showSnackbar(it)
            vm.clearError()
        }
    }

    val d = state.details
    Scaffold(
        containerColor = p.bg,
        snackbarHost = { SnackbarHost(snackbar) },
        topBar = {
            Column {
                AlbumTopBar(
                    title = stringResource(R.string.share_title),
                    subtitle = d?.let { albumName(it) },
                    onBack = onBack,
                )
                if (state.busy) LinearProgressIndicator(Modifier.fillMaxWidth(), color = p.accent2, trackColor = p.panel)
            }
        },
    ) { padding ->
        Box(Modifier.fillMaxSize().padding(padding)) {
            when {
                !state.loaded -> CircularProgressIndicator(color = p.accent2, modifier = Modifier.align(Alignment.Center))
                d == null || !d.owned -> Text(
                    stringResource(R.string.share_owner_only),
                    color = p.muted,
                    modifier = Modifier.align(Alignment.Center).padding(Spacing.section),
                )
                else -> LazyColumn(
                    Modifier.fillMaxSize().testTag("share_list"),
                    contentPadding = androidx.compose.foundation.layout.PaddingValues(horizontal = Spacing.xl, vertical = Spacing.md),
                    verticalArrangement = Arrangement.spacedBy(Spacing.md),
                ) {
                    item(key = "people", contentType = "label") { SectionLabel(stringResource(R.string.share_people)) }
                    item(key = "add", contentType = "add") { AddPerson(state, vm) }
                    if (d.sharees.isEmpty()) {
                        item(key = "nobody", contentType = "empty") {
                            Text(stringResource(R.string.share_nobody), style = MaterialTheme.typography.bodyMedium, color = p.muted)
                        }
                    }
                    items(d.sharees, key = { "s${it.userId}-${it.email}" }, contentType = { "sharee" }) { s ->
                        ShareeRow(s, busy = state.busy, onRole = { vm.changeRole(s.email, it) }, onRemove = { vm.unshare(s.email) })
                    }
                    item(key = "link-label", contentType = "label") { SectionLabel(stringResource(R.string.share_public_link)) }
                    item(key = "link", contentType = "link") {
                        LinkSection(
                            album = d,
                            busy = state.busy,
                            vm = vm,
                            onCopy = { url ->
                                copyLink(context, url)
                                // Android 13+ confirms copies itself.
                                if (Build.VERSION.SDK_INT < Build.VERSION_CODES.TIRAMISU) {
                                    scope.launch { snackbar.showSnackbar(copied) }
                                }
                            },
                            onShare = { url -> shareLink(context, albumNameFor(context, d), url) },
                        )
                    }
                }
            }
        }
    }
}

private fun albumNameFor(context: Context, d: AlbumDetails): String =
    d.collection.name.ifBlank { context.getString(R.string.album_untitled) }

@Composable
private fun AddPerson(state: ShareState, vm: ShareViewModel) {
    var email by rememberSaveable { mutableStateOf("") }
    var role by rememberSaveable { mutableStateOf(ShareRole.Viewer) }
    LaunchedEffect(state.addedEmail) {
        if (state.addedEmail != null) {
            email = ""
            vm.addedEmailConsumed()
        }
    }
    Column(verticalArrangement = Arrangement.spacedBy(Spacing.md)) {
        PvTextField(
            value = email,
            onValueChange = { email = it },
            label = stringResource(R.string.share_email),
            placeholder = stringResource(R.string.share_email_hint),
            keyboardType = KeyboardType.Email,
            imeAction = ImeAction.Done,
            onIme = { vm.share(email, role) },
            modifier = Modifier.testTag("share_email"),
        )
        Row(horizontalArrangement = Arrangement.spacedBy(Spacing.md), verticalAlignment = Alignment.CenterVertically) {
            RoleChip(ShareRole.Viewer, role) { role = it }
            RoleChip(ShareRole.Collaborator, role) { role = it }
            Spacer(Modifier.weight(1f))
            PrimaryButton(
                text = stringResource(R.string.share_add),
                onClick = { vm.share(email, role) },
                enabled = email.contains('@') && !state.busy,
                modifier = Modifier.testTag("share_add"),
            )
        }
        Text(
            stringResource(if (role == ShareRole.Viewer) R.string.share_role_viewer_help else R.string.share_role_collaborator_help),
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.palette.muted,
        )
    }
}

@Composable
private fun RoleChip(r: ShareRole, current: ShareRole, onPick: (ShareRole) -> Unit) {
    val p = MaterialTheme.palette
    FilterChip(
        selected = r == current,
        onClick = { onPick(r) },
        label = { Text(roleLabel(r)) },
        colors = FilterChipDefaults.filterChipColors(
            containerColor = p.panel,
            labelColor = p.muted,
            selectedContainerColor = p.panel2,
            selectedLabelColor = p.text,
        ),
    )
}

@Composable
private fun roleLabel(r: ShareRole): String = stringResource(
    when (r) {
        ShareRole.Viewer -> R.string.share_role_viewer
        ShareRole.Collaborator -> R.string.share_role_collaborator
        ShareRole.Admin -> R.string.share_role_admin
        ShareRole.Owner -> R.string.share_role_owner
        ShareRole.Unknown -> R.string.share_role_viewer
    },
)

@Composable
private fun ShareeRow(s: Sharee, busy: Boolean, onRole: (ShareRole) -> Unit, onRemove: () -> Unit) {
    val p = MaterialTheme.palette
    val shape = RoundedCornerShape(EncoreRadius.tile)
    var menu by remember { mutableStateOf(false) }
    var confirm by remember { mutableStateOf(false) }
    Row(
        Modifier
            .fillMaxWidth()
            .clip(shape)
            .background(p.panel, shape)
            .border(1.dp, p.border, shape)
            .padding(start = Spacing.lg, end = Spacing.xs, top = Spacing.xs, bottom = Spacing.xs),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Icon(painterResource(R.drawable.ic_account), contentDescription = null, tint = p.accent2, modifier = Modifier.size(22.dp))
        Spacer(Modifier.width(Spacing.lg))
        Column(Modifier.weight(1f)) {
            Text(s.email, style = MaterialTheme.typography.bodyLarge, color = p.text, maxLines = 1, overflow = TextOverflow.Ellipsis)
            Box {
                Text(
                    roleLabel(s.role),
                    style = MaterialTheme.typography.bodySmall,
                    color = p.accent2,
                    modifier = Modifier.clip(RoundedCornerShape(EncoreRadius.small)).clickable(enabled = !busy) { menu = true }.padding(vertical = Spacing.xxs),
                )
                DropdownMenu(expanded = menu, onDismissRequest = { menu = false }) {
                    listOf(ShareRole.Viewer, ShareRole.Collaborator).forEach { r ->
                        DropdownMenuItem(text = { Text(roleLabel(r)) }, onClick = { menu = false; if (r != s.role) onRole(r) })
                    }
                }
            }
        }
        IconButton(onClick = { confirm = true }, enabled = !busy) {
            Icon(painterResource(R.drawable.ic_close), contentDescription = stringResource(R.string.share_remove, s.email), tint = p.muted)
        }
    }
    if (confirm) {
        ConfirmDialog(
            title = stringResource(R.string.share_remove_title, s.email),
            body = stringResource(R.string.share_remove_body),
            confirm = stringResource(R.string.share_remove_action),
            danger = true,
            onConfirm = { confirm = false; onRemove() },
            onDismiss = { confirm = false },
        )
    }
}

@Composable
private fun LinkSection(
    album: AlbumDetails,
    busy: Boolean,
    vm: ShareViewModel,
    onCopy: (String) -> Unit,
    onShare: (String) -> Unit,
) {
    val p = MaterialTheme.palette
    val link = album.publicLink
    if (link == null) {
        Column(verticalArrangement = Arrangement.spacedBy(Spacing.md)) {
            Text(stringResource(R.string.share_link_none), style = MaterialTheme.typography.bodyMedium, color = p.muted)
            PrimaryButton(
                text = stringResource(R.string.share_link_create),
                onClick = vm::createLink,
                enabled = !busy,
                modifier = Modifier.fillMaxWidth().testTag("share_link_create"),
            )
        }
        return
    }
    var removing by remember { mutableStateOf(false) }
    var settingPassword by remember { mutableStateOf(false) }
    val shape = RoundedCornerShape(EncoreRadius.tile)
    Column(verticalArrangement = Arrangement.spacedBy(Spacing.md)) {
        Text(
            link.url,
            style = MaterialTheme.typography.bodySmall.copy(fontFamily = FontFamily.Monospace),
            color = p.text,
            maxLines = 2,
            overflow = TextOverflow.Ellipsis,
            modifier = Modifier
                .fillMaxWidth()
                .clip(shape)
                .background(p.panel, shape)
                .border(1.dp, p.border, shape)
                .padding(Spacing.lg)
                .testTag("share_link_url"),
        )
        Row(horizontalArrangement = Arrangement.spacedBy(Spacing.md)) {
            SecondaryButton(stringResource(R.string.share_link_copy), onClick = { onCopy(link.url) }, modifier = Modifier.weight(1f))
            PrimaryButton(stringResource(R.string.share_link_send), onClick = { onShare(link.url) }, modifier = Modifier.weight(1f))
        }
        Text(stringResource(R.string.share_link_note), style = MaterialTheme.typography.bodySmall, color = p.muted)

        SettingRow(stringResource(R.string.share_link_expiry), expiryLabel(link)) { close ->
            LinkExpiry.entries.forEach { e ->
                DropdownMenuItem(text = { Text(expiryChoice(e)) }, onClick = { close(); vm.setExpiry(e) })
            }
        }
        SettingRow(stringResource(R.string.share_link_devices), deviceLabel(link.deviceLimit)) { close ->
            DeviceLimits.forEach { n ->
                DropdownMenuItem(text = { Text(deviceLabel(n)) }, onClick = { close(); vm.setDeviceLimit(n) })
            }
        }
        SwitchRow(stringResource(R.string.share_link_downloads), link.enableDownload, enabled = !busy) { vm.setDownloads(it) }
        SwitchRow(stringResource(R.string.share_link_collect), link.enableCollect, enabled = !busy) { vm.setCollect(it) }
        SwitchRow(stringResource(R.string.share_link_password), link.passwordEnabled, enabled = !busy) { on ->
            if (on) settingPassword = true else vm.removePassword()
        }
        TextButton(onClick = { removing = true }, enabled = !busy) {
            Text(stringResource(R.string.share_link_remove), color = p.err)
        }
    }
    if (removing) {
        ConfirmDialog(
            title = stringResource(R.string.share_link_remove_title),
            body = stringResource(R.string.share_link_remove_body),
            confirm = stringResource(R.string.share_link_remove),
            danger = true,
            onConfirm = { removing = false; vm.removeLink() },
            onDismiss = { removing = false },
        )
    }
    if (settingPassword) {
        PasswordDialog(onSet = { settingPassword = false; vm.setPassword(it) }, onDismiss = { settingPassword = false })
    }
}

@Composable
private fun SettingRow(title: String, value: String, menu: @Composable (close: () -> Unit) -> Unit) {
    val p = MaterialTheme.palette
    var open by remember { mutableStateOf(false) }
    Box {
        Row(
            Modifier
                .fillMaxWidth()
                .clip(RoundedCornerShape(EncoreRadius.control))
                .clickable { open = true }
                .padding(vertical = Spacing.base),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Text(title, style = MaterialTheme.typography.bodyLarge, color = p.text, modifier = Modifier.weight(1f))
            Text(value, style = MaterialTheme.typography.bodyMedium, color = p.accent2)
            Icon(painterResource(R.drawable.ic_chevron_right), contentDescription = null, tint = p.muted, modifier = Modifier.size(20.dp))
        }
        DropdownMenu(expanded = open, onDismissRequest = { open = false }) { menu { open = false } }
    }
}

@Composable
private fun SwitchRow(title: String, checked: Boolean, enabled: Boolean, onChange: (Boolean) -> Unit) {
    val p = MaterialTheme.palette
    Row(Modifier.fillMaxWidth().padding(vertical = Spacing.xs), verticalAlignment = Alignment.CenterVertically) {
        Text(title, style = MaterialTheme.typography.bodyLarge, color = p.text, modifier = Modifier.weight(1f))
        Switch(
            checked = checked,
            onCheckedChange = onChange,
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
private fun PasswordDialog(onSet: (String) -> Unit, onDismiss: () -> Unit) {
    val p = MaterialTheme.palette
    var pw by rememberSaveable { mutableStateOf("") }
    AlertDialog(
        onDismissRequest = onDismiss,
        containerColor = p.bg2,
        title = { Text(stringResource(R.string.share_link_password_title), color = p.text) },
        text = {
            Column(verticalArrangement = Arrangement.spacedBy(Spacing.md)) {
                Text(stringResource(R.string.share_link_password_body), color = p.muted, style = MaterialTheme.typography.bodyMedium)
                PasswordField(value = pw, onValueChange = { pw = it }, label = stringResource(R.string.signin_password), onIme = { if (pw.isNotEmpty()) onSet(pw) })
            }
        },
        confirmButton = { TextButton(onClick = { onSet(pw) }, enabled = pw.isNotEmpty()) { Text(stringResource(R.string.albums_save), color = p.accent2) } },
        dismissButton = { TextButton(onClick = onDismiss) { Text(stringResource(R.string.albums_cancel), color = p.muted) } },
    )
}

@Composable
private fun expiryLabel(link: PublicLink): String =
    if (link.validTillUs <= 0) {
        stringResource(R.string.share_expiry_never)
    } else {
        val date = DateFormat.getDateTimeInstance(DateFormat.MEDIUM, DateFormat.SHORT).format(Date(link.validTillUs / 1000))
        stringResource(R.string.share_expiry_until, date)
    }

@Composable
private fun expiryChoice(e: LinkExpiry): String = stringResource(
    when (e) {
        LinkExpiry.Never -> R.string.share_expiry_never
        LinkExpiry.Hour -> R.string.share_expiry_hour
        LinkExpiry.Day -> R.string.share_expiry_day
        LinkExpiry.Week -> R.string.share_expiry_week
        LinkExpiry.Month -> R.string.share_expiry_month
        LinkExpiry.Year -> R.string.share_expiry_year
    },
)

@Composable
private fun deviceLabel(n: Int): String =
    if (n <= 0) stringResource(R.string.share_devices_unlimited) else stringResource(R.string.share_devices_n, n)

/** Copies the link, marked sensitive: it carries the album's key, so the clipboard preview hides it. */
private fun copyLink(context: Context, url: String) {
    val cm = context.getSystemService(ClipboardManager::class.java) ?: return
    val clip = ClipData.newPlainText(context.getString(R.string.share_public_link), url)
    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
        clip.description.extras = PersistableBundle().apply { putBoolean(ClipDescription.EXTRA_IS_SENSITIVE, true) }
    }
    cm.setPrimaryClip(clip)
}

private fun shareLink(context: Context, albumName: String, url: String) {
    val send = Intent(Intent.ACTION_SEND).apply {
        type = "text/plain"
        putExtra(Intent.EXTRA_SUBJECT, albumName)
        putExtra(Intent.EXTRA_TEXT, url)
    }
    context.startActivity(Intent.createChooser(send, context.getString(R.string.share_link_send)).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
}
