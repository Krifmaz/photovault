@file:OptIn(ExperimentalMaterial3Api::class)

package app.photovault.android.ui.settings

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBar
import androidx.compose.material3.TopAppBarDefaults
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import app.photovault.android.R
import app.photovault.android.ui.components.ProblemText
import app.photovault.android.ui.components.SectionLabel
import app.photovault.android.ui.theme.EncoreRadius
import app.photovault.android.ui.theme.Spacing
import app.photovault.android.ui.theme.palette

/**
 * Settings for machine learning (faces and magic search): the on/off switch shared with the Ente apps,
 * a privacy note, the indexing status and "index only while charging".
 */
@Composable
fun MlSettingsScreen(vm: MlSettingsViewModel, onBack: () -> Unit) {
    val state by vm.state.collectAsStateWithLifecycle()
    val p = MaterialTheme.palette
    Scaffold(
        containerColor = p.bg,
        topBar = {
            TopAppBar(
                colors = TopAppBarDefaults.topAppBarColors(containerColor = p.bg, titleContentColor = p.text),
                title = { Text(stringResource(R.string.ml_settings_title), style = MaterialTheme.typography.titleLarge) },
                navigationIcon = {
                    IconButton(onClick = onBack) {
                        Icon(painterResource(R.drawable.ic_arrow_back), contentDescription = stringResource(R.string.back), tint = p.text)
                    }
                },
            )
        },
    ) { padding ->
        Column(
            Modifier
                .fillMaxSize()
                .padding(padding)
                .verticalScroll(rememberScrollState())
                .padding(horizontal = Spacing.xl, vertical = Spacing.md),
            verticalArrangement = Arrangement.spacedBy(Spacing.md),
        ) {
            val enabled = state.status?.enabled == true
            SettingRow(
                title = stringResource(R.string.ml_settings_enable),
                body = stringResource(R.string.ml_settings_enable_body),
                checked = enabled,
                enabled = state.status != null && !state.busy,
                onChange = vm::setEnabled,
                tag = "ml_switch",
            )
            Text(
                stringResource(R.string.ml_settings_privacy),
                style = MaterialTheme.typography.bodySmall,
                color = p.muted,
                modifier = Modifier.padding(horizontal = Spacing.xs),
            )
            state.error?.let { ProblemText(stringResource(R.string.ml_settings_switch_failed, it)) }

            if (enabled) {
                SectionLabel(stringResource(R.string.ml_settings_status))
                StatusPanel(statusLines(state.status))
                SectionLabel(stringResource(R.string.ml_settings_indexing))
                SettingRow(
                    title = stringResource(R.string.ml_settings_charging),
                    body = stringResource(R.string.ml_settings_charging_body),
                    checked = state.onlyWhileCharging,
                    enabled = true,
                    onChange = vm::setOnlyWhileCharging,
                    tag = "ml_charging_switch",
                )
            }
        }
    }
}

@Composable
private fun SettingRow(title: String, body: String, checked: Boolean, enabled: Boolean, onChange: (Boolean) -> Unit, tag: String) {
    val p = MaterialTheme.palette
    val shape = RoundedCornerShape(EncoreRadius.tile)
    Row(
        Modifier
            .fillMaxWidth()
            .widthIn(max = 560.dp)
            .clip(shape)
            .background(p.panel, shape)
            .border(1.dp, p.border, shape)
            .padding(horizontal = Spacing.lg, vertical = Spacing.base),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(Spacing.md),
    ) {
        Column(Modifier.weight(1f)) {
            Text(title, style = MaterialTheme.typography.titleMedium, color = p.text)
            Text(body, style = MaterialTheme.typography.bodySmall, color = p.muted)
        }
        Switch(checked = checked, onCheckedChange = onChange, enabled = enabled, modifier = Modifier.testTag(tag))
    }
}

@Composable
private fun StatusPanel(lines: List<MlStatusLine>) {
    val p = MaterialTheme.palette
    val shape = RoundedCornerShape(EncoreRadius.tile)
    Column(
        Modifier
            .fillMaxWidth()
            .clip(shape)
            .background(p.panel, shape)
            .border(1.dp, p.border, shape)
            .padding(horizontal = Spacing.lg, vertical = Spacing.base)
            .testTag("ml_status"),
        verticalArrangement = Arrangement.spacedBy(Spacing.md),
    ) {
        if (lines.isEmpty()) {
            Text(stringResource(R.string.ml_status_waiting), style = MaterialTheme.typography.bodyMedium, color = p.muted)
        }
        for (line in lines) {
            when (line) {
                MlStatusLine.NoInference -> Text(stringResource(R.string.ml_status_no_inference), style = MaterialTheme.typography.bodyMedium, color = p.muted)
                is MlStatusLine.DownloadingModels -> Column(verticalArrangement = Arrangement.spacedBy(Spacing.xs)) {
                    Text(
                        stringResource(R.string.ml_status_models, line.doneMb, line.totalMb),
                        style = MaterialTheme.typography.bodyMedium,
                        color = p.text,
                    )
                    LinearProgressIndicator(
                        progress = { if (line.totalMb > 0) (line.doneMb.toFloat() / line.totalMb).coerceIn(0f, 1f) else 0f },
                        modifier = Modifier.fillMaxWidth(),
                        color = p.accent2,
                        trackColor = p.panel2,
                    )
                }
                is MlStatusLine.Indexed -> Column(verticalArrangement = Arrangement.spacedBy(Spacing.xs)) {
                    Text(
                        stringResource(R.string.ml_status_indexed, line.done, line.total),
                        style = MaterialTheme.typography.bodyMedium,
                        color = p.text,
                    )
                    LinearProgressIndicator(
                        progress = { (line.done.toFloat() / line.total).coerceIn(0f, 1f) },
                        modifier = Modifier.fillMaxWidth(),
                        color = p.accent2,
                        trackColor = p.panel2,
                    )
                }
                MlStatusLine.Running -> Text(stringResource(R.string.ml_status_running), style = MaterialTheme.typography.bodyMedium, color = p.muted)
                is MlStatusLine.Problem -> ProblemText(stringResource(R.string.ml_status_error, line.message))
            }
        }
    }
}
