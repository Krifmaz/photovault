package app.photovault.android.ui.components

import androidx.annotation.DrawableRes
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.OutlinedTextFieldDefaults
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.draw.clip
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.text.input.VisualTransformation
import androidx.compose.ui.unit.dp
import app.photovault.android.R
import app.photovault.android.ui.theme.EncoreRadius
import app.photovault.android.ui.theme.Spacing
import app.photovault.android.ui.theme.palette

/** Encore's primary button: the accent gradient with onAccent text (encore-tokens.json "primaryButton"). */
@Composable
fun PrimaryButton(
    text: String,
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
    enabled: Boolean = true,
    loading: Boolean = false,
) {
    val p = MaterialTheme.palette
    val shape = RoundedCornerShape(EncoreRadius.control)
    Box(
        modifier = modifier
            .alpha(if (enabled) 1f else 0.5f)
            .heightIn(min = 48.dp)
            .clip(shape)
            .background(p.gradient, shape)
            .clickable(enabled = enabled && !loading, role = Role.Button, onClick = onClick)
            .padding(horizontal = Spacing.xl, vertical = Spacing.base),
        contentAlignment = Alignment.Center,
    ) {
        if (loading) {
            CircularProgressIndicator(modifier = Modifier.size(20.dp), color = p.onAccent, strokeWidth = 2.dp)
        } else {
            Text(text, color = p.onAccent, style = MaterialTheme.typography.labelLarge)
        }
    }
}

/** Secondary action: a panel-coloured button with a border. */
@Composable
fun SecondaryButton(text: String, onClick: () -> Unit, modifier: Modifier = Modifier, enabled: Boolean = true) {
    val p = MaterialTheme.palette
    val shape = RoundedCornerShape(EncoreRadius.control)
    Box(
        modifier = modifier
            .alpha(if (enabled) 1f else 0.5f)
            .heightIn(min = 48.dp)
            .clip(shape)
            .background(p.panel2, shape)
            .border(1.dp, p.border, shape)
            .clickable(enabled = enabled, role = Role.Button, onClick = onClick)
            .padding(horizontal = Spacing.xl, vertical = Spacing.base),
        contentAlignment = Alignment.Center,
    ) {
        Text(text, color = p.text, style = MaterialTheme.typography.labelLarge)
    }
}

@Composable
fun LinkButton(text: String, onClick: () -> Unit, modifier: Modifier = Modifier, enabled: Boolean = true) {
    TextButton(onClick = onClick, modifier = modifier, enabled = enabled) {
        Text(text, color = MaterialTheme.palette.accent2, style = MaterialTheme.typography.labelLarge)
    }
}

/** Encore input: panel background, 10 dp corners, border that turns accent on focus. */
@Composable
fun PvTextField(
    value: String,
    onValueChange: (String) -> Unit,
    label: String,
    modifier: Modifier = Modifier,
    placeholder: String? = null,
    keyboardType: KeyboardType = KeyboardType.Text,
    imeAction: ImeAction = ImeAction.Next,
    onIme: (() -> Unit)? = null,
    isError: Boolean = false,
    enabled: Boolean = true,
    singleLine: Boolean = true,
    minLines: Int = 1,
    trailing: (@Composable () -> Unit)? = null,
) {
    val p = MaterialTheme.palette
    OutlinedTextField(
        value = value,
        onValueChange = onValueChange,
        modifier = modifier.fillMaxWidth(),
        label = { Text(label) },
        placeholder = placeholder?.let { { Text(it, color = p.faint) } },
        singleLine = singleLine,
        minLines = minLines,
        enabled = enabled,
        isError = isError,
        trailingIcon = trailing,
        shape = RoundedCornerShape(EncoreRadius.control),
        keyboardOptions = KeyboardOptions(keyboardType = keyboardType, imeAction = imeAction),
        keyboardActions = KeyboardActions(onAny = { onIme?.invoke() }),
        colors = fieldColors(),
    )
}

@Composable
fun PasswordField(
    value: String,
    onValueChange: (String) -> Unit,
    label: String,
    modifier: Modifier = Modifier,
    imeAction: ImeAction = ImeAction.Done,
    onIme: (() -> Unit)? = null,
    isError: Boolean = false,
    enabled: Boolean = true,
) {
    var visible by rememberSaveable { mutableStateOf(false) }
    OutlinedTextField(
        value = value,
        onValueChange = onValueChange,
        modifier = modifier.fillMaxWidth(),
        label = { Text(label) },
        singleLine = true,
        enabled = enabled,
        isError = isError,
        visualTransformation = if (visible) VisualTransformation.None else PasswordVisualTransformation(),
        trailingIcon = {
            IconButton(onClick = { visible = !visible }) {
                Icon(
                    painterResource(if (visible) R.drawable.ic_visibility_off else R.drawable.ic_visibility),
                    contentDescription = stringResource(if (visible) R.string.password_hide else R.string.password_show),
                )
            }
        },
        shape = RoundedCornerShape(EncoreRadius.control),
        keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Password, imeAction = imeAction),
        keyboardActions = KeyboardActions(onAny = { onIme?.invoke() }),
        colors = fieldColors(),
    )
}

@Composable
private fun fieldColors() = MaterialTheme.palette.let { p ->
    OutlinedTextFieldDefaults.colors(
        focusedContainerColor = p.panel,
        unfocusedContainerColor = p.panel,
        disabledContainerColor = p.panel,
        focusedBorderColor = p.accent2,
        unfocusedBorderColor = p.border,
        focusedLabelColor = p.accent2,
        unfocusedLabelColor = p.muted,
        cursorColor = p.accent2,
        focusedTextColor = p.text,
        unfocusedTextColor = p.text,
        errorBorderColor = p.err,
    )
}

/** A problem sentence under a form, in the error colour on a tinted panel. */
@Composable
fun ProblemText(text: String, modifier: Modifier = Modifier) {
    val p = MaterialTheme.palette
    Row(
        modifier = modifier
            .fillMaxWidth()
            .clip(RoundedCornerShape(EncoreRadius.control))
            .background(p.err.copy(alpha = 0.12f))
            .padding(horizontal = Spacing.lg, vertical = Spacing.base),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(Spacing.md),
    ) {
        Icon(painterResource(R.drawable.ic_warning), contentDescription = null, tint = p.err, modifier = Modifier.size(18.dp))
        Text(text, color = p.text, style = MaterialTheme.typography.bodyMedium)
    }
}

/** The "Beta" pill next to the app name. */
@Composable
fun BetaBadge(modifier: Modifier = Modifier) {
    val p = MaterialTheme.palette
    Text(
        text = stringResource(R.string.beta),
        modifier = modifier
            .clip(RoundedCornerShape(EncoreRadius.pill))
            .background(p.accent2.copy(alpha = 0.18f))
            .padding(horizontal = Spacing.md, vertical = Spacing.xxs),
        color = p.accent2,
        style = MaterialTheme.typography.labelSmall,
    )
}

/** The Photovault glyph (branding/glyph.svg). */
@Composable
fun Logo(modifier: Modifier = Modifier) {
    Image(painterResource(R.drawable.ic_glyph), contentDescription = null, modifier = modifier)
}

/** Screen title block used by the connect and sign-in screens. */
@Composable
fun ScreenHeader(title: String, subtitle: String?, modifier: Modifier = Modifier, showBrand: Boolean = true) {
    val p = MaterialTheme.palette
    Column(modifier = modifier.fillMaxWidth(), verticalArrangement = Arrangement.spacedBy(Spacing.md)) {
        if (showBrand) {
            Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(Spacing.base)) {
                Logo(Modifier.size(40.dp))
                Text(stringResource(R.string.app_name), style = MaterialTheme.typography.titleMedium, color = p.text)
                BetaBadge()
            }
        }
        Text(title, style = MaterialTheme.typography.headlineMedium, color = p.text, modifier = Modifier.padding(top = Spacing.md))
        if (subtitle != null) Text(subtitle, style = MaterialTheme.typography.bodyMedium, color = p.muted)
    }
}

/** A tappable list row on a panel: icon, title, subtitle and an optional trailing slot. */
@Composable
fun PanelRow(
    title: String,
    subtitle: String?,
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
    @DrawableRes icon: Int? = null,
    trailing: (@Composable () -> Unit)? = null,
) {
    val p = MaterialTheme.palette
    val shape = RoundedCornerShape(EncoreRadius.tile)
    Row(
        modifier = modifier
            .fillMaxWidth()
            .clip(shape)
            .background(p.panel, shape)
            .border(1.dp, p.border, shape)
            .clickable(onClick = onClick)
            .padding(horizontal = Spacing.lg, vertical = Spacing.base),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(Spacing.lg),
    ) {
        if (icon != null) Icon(painterResource(icon), contentDescription = null, tint = p.accent2, modifier = Modifier.size(22.dp))
        Column(Modifier.weight(1f)) {
            Text(title, style = MaterialTheme.typography.titleMedium, color = p.text)
            if (subtitle != null) Text(subtitle, style = MaterialTheme.typography.bodySmall, color = p.muted)
        }
        trailing?.invoke()
    }
}

@Composable
fun SectionLabel(text: String, modifier: Modifier = Modifier) {
    Text(
        text.uppercase(),
        modifier = modifier.padding(top = Spacing.xl, bottom = Spacing.xs),
        style = MaterialTheme.typography.labelSmall,
        color = MaterialTheme.palette.muted,
    )
}
