package app.photovault.android.ui.hidden

import android.app.Activity
import android.app.KeyguardManager
import android.os.Build
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.grid.GridCells
import androidx.compose.foundation.lazy.grid.GridItemSpan
import androidx.compose.foundation.lazy.grid.LazyVerticalGrid
import androidx.compose.foundation.lazy.grid.items
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import app.photovault.android.R
import app.photovault.android.ui.albums.AlbumTile
import app.photovault.android.ui.albums.AlbumTopBar
import app.photovault.android.ui.components.PrimaryButton
import app.photovault.android.ui.theme.Spacing
import app.photovault.android.ui.theme.palette

/** Hidden albums and hidden photos, behind the device lock. */
@Composable
fun HiddenScreen(vm: HiddenViewModel, onOpenAlbum: (Long) -> Unit, onBack: () -> Unit) {
    val state by vm.state.collectAsStateWithLifecycle()
    val p = MaterialTheme.palette
    val context = LocalContext.current
    val snackbar = remember { SnackbarHostState() }
    val secure = remember { DeviceLock.isSecure(context) }
    // Bumped by "Unlock" to show the prompt again.
    var attempt by rememberSaveable { mutableIntStateOf(0) }
    var failed by remember { mutableStateOf(false) }
    val title = stringResource(R.string.hidden_unlock_title)
    val subtitle = stringResource(R.string.hidden_unlock_subtitle)

    val keyguard = rememberLauncherForActivityResult(ActivityResultContracts.StartActivityForResult()) { r ->
        if (r.resultCode == Activity.RESULT_OK) vm.unlocked() else failed = true
    }

    if (!state.unlocked) {
        if (!secure) {
            // No screen lock: nothing to check. Say so (below) and open.
            LaunchedEffect(Unit) { vm.unlocked() }
        } else if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
            DisposableEffect(attempt) {
                failed = false
                val cancel = DeviceLock.authenticate(context, title, subtitle) { ok -> if (ok) vm.unlocked() else failed = true }
                onDispose { cancel.cancel() }
            }
        } else {
            LaunchedEffect(attempt) {
                failed = false
                @Suppress("DEPRECATION")
                val intent = context.getSystemService(KeyguardManager::class.java)?.createConfirmDeviceCredentialIntent(title, subtitle)
                if (intent != null) keyguard.launch(intent) else vm.unlocked()
            }
        }
    }

    // Back from a hidden album (unhidden, renamed, emptied): re-read.
    LaunchedEffect(vm) { vm.refresh() }
    LaunchedEffect(state.error) {
        state.error?.let {
            snackbar.showSnackbar(it)
            vm.clearError()
        }
    }

    Scaffold(
        containerColor = p.bg,
        snackbarHost = { SnackbarHost(snackbar) },
        topBar = { AlbumTopBar(title = stringResource(R.string.hidden_title), subtitle = null, onBack = onBack) },
    ) { padding ->
        Box(Modifier.fillMaxSize().padding(padding)) {
            when {
                !state.unlocked -> Column(
                    Modifier.align(Alignment.Center).padding(Spacing.section),
                    horizontalAlignment = Alignment.CenterHorizontally,
                    verticalArrangement = Arrangement.spacedBy(Spacing.lg),
                ) {
                    Icon(painterResource(R.drawable.ic_lock), contentDescription = null, tint = p.accent2, modifier = Modifier.size(40.dp))
                    Text(stringResource(R.string.hidden_locked), style = MaterialTheme.typography.titleMedium, color = p.text, textAlign = TextAlign.Center)
                    if (failed) {
                        PrimaryButton(stringResource(R.string.hidden_unlock), onClick = { attempt++ }, modifier = Modifier.testTag("hidden_unlock"))
                    }
                }
                !state.loaded -> CircularProgressIndicator(color = p.accent2, modifier = Modifier.align(Alignment.Center))
                else -> LazyVerticalGrid(
                    columns = GridCells.Adaptive(150.dp),
                    contentPadding = PaddingValues(Spacing.md),
                    modifier = Modifier.fillMaxSize().testTag("hidden_grid"),
                ) {
                    item(key = "note", span = { GridItemSpan(maxLineSpan) }, contentType = "note") {
                        Text(
                            stringResource(if (secure) R.string.hidden_note else R.string.hidden_no_lock),
                            style = MaterialTheme.typography.bodySmall,
                            color = p.muted,
                            modifier = Modifier.fillMaxWidth().padding(Spacing.xs),
                        )
                    }
                    items(state.albums, key = { "a${it.id}" }, contentType = { "album" }) { a ->
                        AlbumTile(a, onClick = { onOpenAlbum(a.id) })
                    }
                    if (state.albums.isEmpty()) {
                        item(key = "empty", span = { GridItemSpan(maxLineSpan) }, contentType = "empty") {
                            Text(
                                stringResource(R.string.hidden_empty),
                                style = MaterialTheme.typography.bodyMedium,
                                color = p.muted,
                                textAlign = TextAlign.Center,
                                modifier = Modifier.fillMaxWidth().padding(Spacing.section),
                            )
                        }
                    }
                }
            }
        }
    }
}
