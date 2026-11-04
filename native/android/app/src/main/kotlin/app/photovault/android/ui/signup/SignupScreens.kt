package app.photovault.android.ui.signup

import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import android.os.PersistableBundle
import androidx.activity.compose.BackHandler
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawing
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.selection.toggleable
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Checkbox
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import app.photovault.android.R
import app.photovault.android.ui.components.LinkButton
import app.photovault.android.ui.components.PasswordField
import app.photovault.android.ui.components.PrimaryButton
import app.photovault.android.ui.components.ProblemText
import app.photovault.android.ui.components.PvTextField
import app.photovault.android.ui.components.ScreenHeader
import app.photovault.android.ui.components.SecondaryButton
import app.photovault.android.ui.theme.EncoreRadius
import app.photovault.android.ui.theme.Spacing
import app.photovault.android.ui.theme.palette

/** Routes the view model's sign-up events to navigation; one collector per sign-up screen. */
@Composable
fun SignupEvents(
    vm: SignupViewModel,
    onPassword: () -> Unit,
    onCode: () -> Unit,
    onRecoveryKey: () -> Unit,
    onDone: () -> Unit,
    onRestart: () -> Unit,
) {
    val password by rememberUpdatedState(onPassword)
    val code by rememberUpdatedState(onCode)
    val recovery by rememberUpdatedState(onRecoveryKey)
    val done by rememberUpdatedState(onDone)
    val restart by rememberUpdatedState(onRestart)
    LaunchedEffect(vm) {
        vm.eventFlow.collect { e ->
            when (e) {
                SignupEvent.Password -> password()
                SignupEvent.Code -> code()
                SignupEvent.RecoveryKey -> recovery()
                SignupEvent.Done -> done()
                SignupEvent.Restart -> restart()
            }
        }
    }
}

/** Insets, scrolling, a back arrow and a centred column of at most 560 dp (as the sign-in screens). */
@Composable
private fun SignupFrame(onBack: (() -> Unit)?, content: @Composable ColumnScope.() -> Unit) {
    Column(
        modifier = Modifier
            .fillMaxSize()
            .background(MaterialTheme.palette.bg)
            .windowInsetsPadding(WindowInsets.safeDrawing)
            .imePadding()
            .verticalScroll(rememberScrollState()),
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        if (onBack != null) {
            IconButton(onClick = onBack, modifier = Modifier.align(Alignment.Start).padding(Spacing.xs)) {
                Icon(painterResource(R.drawable.ic_arrow_back), contentDescription = stringResource(R.string.back))
            }
        }
        Column(
            modifier = Modifier
                .widthIn(max = 560.dp)
                .fillMaxWidth()
                .padding(horizontal = Spacing.xxl, vertical = Spacing.xl),
            verticalArrangement = Arrangement.spacedBy(Spacing.lg),
            content = content,
        )
    }
}

@Composable
private fun problemText(problem: SignupProblem): String = when (problem) {
    SignupProblem.MissingFields -> stringResource(R.string.signup_missing)
    SignupProblem.InvalidEmail -> stringResource(R.string.signup_invalid_email)
    SignupProblem.PasswordsDiffer -> stringResource(R.string.recovery_mismatch)
    SignupProblem.PasswordTooShort -> stringResource(R.string.recovery_too_short, SignupViewModel.MIN_PASSWORD)
    SignupProblem.PasswordTooWeak -> stringResource(R.string.signup_too_weak)
    SignupProblem.IncorrectCode -> stringResource(R.string.signin_incorrect_code)
    SignupProblem.AlreadyRegistered -> stringResource(R.string.signup_already_registered)
    SignupProblem.SignupsClosed -> stringResource(R.string.signin_signups_closed)
    SignupProblem.NotSaved -> stringResource(R.string.signup_not_saved)
    is SignupProblem.Unreachable -> stringResource(R.string.connect_unreachable, problem.address)
    is SignupProblem.Other -> problem.message
}

@Composable
fun SignupEmailScreen(vm: SignupViewModel, onBack: () -> Unit) {
    val state by vm.state.collectAsStateWithLifecycle()
    SignupFrame(onBack = onBack) {
        ScreenHeader(
            title = stringResource(R.string.signup_title),
            subtitle = if (state.serverLabel.isNotEmpty()) stringResource(R.string.signup_subtitle, state.serverLabel) else null,
        )
        if (state.signupsAllowed == false) {
            ProblemText(stringResource(R.string.signin_signups_closed))
        } else {
            PvTextField(
                value = state.email,
                onValueChange = vm::onEmailChange,
                label = stringResource(R.string.signin_email),
                keyboardType = KeyboardType.Email,
                imeAction = ImeAction.Done,
                onIme = vm::submitEmail,
                isError = state.problem == SignupProblem.InvalidEmail,
            )
            state.problem?.let { ProblemText(problemText(it)) }
            PrimaryButton(stringResource(R.string.signup_email_action), vm::submitEmail, Modifier.fillMaxWidth())
        }
        LinkButton(stringResource(R.string.signup_have_account), onBack, Modifier.align(Alignment.CenterHorizontally))
    }
}

@Composable
fun SignupPasswordScreen(vm: SignupViewModel, onBack: () -> Unit) {
    val state by vm.state.collectAsStateWithLifecycle()
    var password by remember { mutableStateOf("") } // secrets stay out of saved state
    var confirm by remember { mutableStateOf("") } // secrets stay out of saved state
    val submit = { vm.submitPassword(password, confirm) }
    SignupFrame(onBack = onBack) {
        ScreenHeader(
            title = stringResource(R.string.signup_password_title),
            subtitle = stringResource(R.string.signup_password_subtitle),
            showBrand = false,
        )
        PasswordField(
            value = password,
            onValueChange = { password = it; vm.clearProblem() },
            label = stringResource(R.string.signup_password),
            imeAction = ImeAction.Next,
            enabled = !state.busy,
        )
        if (password.isNotEmpty()) StrengthHint(password)
        PasswordField(
            value = confirm,
            onValueChange = { confirm = it; vm.clearProblem() },
            label = stringResource(R.string.signup_confirm),
            onIme = submit,
            isError = state.problem == SignupProblem.PasswordsDiffer,
            enabled = !state.busy,
        )
        state.problem?.let { ProblemText(problemText(it)) }
        PrimaryButton(stringResource(R.string.signup_password_action), submit, Modifier.fillMaxWidth(), loading = state.busy)
    }
}

@Composable
private fun StrengthHint(password: String) {
    val level = remember(password) { PasswordStrengthEstimator.level(password) }
    val p = MaterialTheme.palette
    val (text, color) = when (level) {
        PasswordStrength.Weak -> stringResource(R.string.signup_strength_weak) to p.err
        PasswordStrength.Moderate -> stringResource(R.string.signup_strength_moderate) to p.warn
        PasswordStrength.Strong -> stringResource(R.string.signup_strength_strong) to p.ok
    }
    Text(text, style = MaterialTheme.typography.bodySmall, color = color)
}

@Composable
fun SignupCodeScreen(vm: SignupViewModel, onBack: () -> Unit) {
    val state by vm.state.collectAsStateWithLifecycle()
    var code by rememberSaveable { mutableStateOf("") }
    SignupFrame(onBack = if (state.busy) null else onBack) {
        ScreenHeader(
            title = stringResource(R.string.code_email_title),
            subtitle = stringResource(R.string.signup_code_subtitle, state.email),
            showBrand = false,
        )
        PvTextField(
            value = code,
            onValueChange = { code = it.filter(Char::isDigit).take(6); vm.clearProblem() },
            label = stringResource(R.string.code_label),
            keyboardType = KeyboardType.NumberPassword,
            imeAction = ImeAction.Done,
            onIme = { vm.verifyCode(code) },
            enabled = !state.busy,
        )
        if (state.makingKeys) {
            Text(stringResource(R.string.signup_making_keys), style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.palette.muted)
        }
        state.problem?.let { ProblemText(problemText(it)) }
        PrimaryButton(stringResource(R.string.code_verify), { vm.verifyCode(code) }, Modifier.fillMaxWidth(), loading = state.busy)
        LinkButton(stringResource(R.string.code_resend), vm::resendCode, Modifier.align(Alignment.CenterHorizontally), enabled = !state.busy)
    }
}

@Composable
fun SignupRecoveryKeyScreen(vm: SignupViewModel, onBack: () -> Unit) {
    val state by vm.state.collectAsStateWithLifecycle()
    val context = LocalContext.current
    var copied by remember { mutableStateOf(false) }
    // Back leaves the recovery key (nothing is uploaded yet): choose the password again.
    BackHandler(enabled = !state.busy, onBack = onBack)
    SignupFrame(onBack = if (state.busy) null else onBack) {
        ScreenHeader(
            title = stringResource(R.string.signup_recovery_title),
            subtitle = stringResource(R.string.signup_recovery_subtitle),
            showBrand = false,
        )
        RecoveryWords(state.recoveryWords)
        val label = stringResource(R.string.signup_recovery_clip_label)
        SecondaryButton(
            text = stringResource(if (copied) R.string.signup_recovery_copied else R.string.signup_recovery_copy),
            onClick = { copySensitive(context, label, state.recoveryWords.joinToString(" ")); copied = true },
            modifier = Modifier.fillMaxWidth(),
            enabled = state.recoveryWords.isNotEmpty(),
        )
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .clip(RoundedCornerShape(EncoreRadius.control))
                .toggleable(value = state.savedConfirmed, enabled = !state.busy, role = Role.Checkbox, onValueChange = vm::setSaved)
                .padding(vertical = Spacing.xs),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(Spacing.md),
        ) {
            Checkbox(checked = state.savedConfirmed, onCheckedChange = null, enabled = !state.busy)
            Text(stringResource(R.string.signup_recovery_saved), style = MaterialTheme.typography.bodyLarge, color = MaterialTheme.palette.text)
        }
        state.problem?.let { ProblemText(problemText(it)) }
        PrimaryButton(
            text = stringResource(R.string.signup_recovery_action),
            onClick = vm::finish,
            modifier = Modifier.fillMaxWidth(),
            enabled = state.savedConfirmed,
            loading = state.busy,
        )
    }
}

/** 24 numbered words in three columns (a fixed, small layout: no lazy list needed). */
@Composable
private fun RecoveryWords(words: List<String>) {
    val p = MaterialTheme.palette
    val description = stringResource(R.string.signup_recovery_words)
    Column(
        modifier = Modifier
            .fillMaxWidth()
            .clip(RoundedCornerShape(EncoreRadius.control))
            .background(p.panel)
            .padding(Spacing.lg)
            .semantics { contentDescription = description },
        verticalArrangement = Arrangement.spacedBy(Spacing.sm),
    ) {
        words.chunked(3).forEachIndexed { row, chunk ->
            Row(Modifier.fillMaxWidth()) {
                chunk.forEachIndexed { col, word ->
                    Row(Modifier.weight(1f), horizontalArrangement = Arrangement.spacedBy(Spacing.xs)) {
                        Text("${row * 3 + col + 1}.", style = MaterialTheme.typography.bodySmall, color = p.faint)
                        Text(word, style = MaterialTheme.typography.bodyMedium, color = p.text, fontFamily = FontFamily.Monospace)
                    }
                }
            }
        }
    }
}

/** Copies the recovery key, marked sensitive so the keyboard's clipboard preview hides it. */
private fun copySensitive(context: Context, label: String, text: String) {
    val cm = context.getSystemService(ClipboardManager::class.java) ?: return
    val clip = ClipData.newPlainText(label, text)
    clip.description.extras = PersistableBundle().apply { putBoolean("android.content.extra.IS_SENSITIVE", true) }
    cm.setPrimaryClip(clip)
}
