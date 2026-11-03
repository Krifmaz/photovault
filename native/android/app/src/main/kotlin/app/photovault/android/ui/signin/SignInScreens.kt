package app.photovault.android.ui.signin

import android.content.ActivityNotFoundException
import android.content.Intent
import android.net.Uri
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawing
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
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
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.res.stringResource
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
import app.photovault.android.ui.theme.Spacing
import app.photovault.android.ui.theme.palette

/** Routes the view model's sign-in events to navigation; one collector per auth screen. */
@Composable
fun SignInEvents(
    vm: SignInViewModel,
    onEmailCode: () -> Unit,
    onTotp: (String) -> Unit,
    onPasskey: (String, String) -> Unit,
    onRecovery: () -> Unit,
    onDone: () -> Unit,
) {
    val emailCode by rememberUpdatedState(onEmailCode)
    val totp by rememberUpdatedState(onTotp)
    val passkey by rememberUpdatedState(onPasskey)
    val recovery by rememberUpdatedState(onRecovery)
    val done by rememberUpdatedState(onDone)
    LaunchedEffect(vm) {
        vm.eventFlow.collect { e ->
            when (e) {
                SignInEvent.EmailCode -> emailCode()
                is SignInEvent.Totp -> totp(e.sessionId)
                is SignInEvent.Passkey -> passkey(e.sessionId, e.url)
                SignInEvent.RecoveryKey -> recovery()
                SignInEvent.Done -> done()
            }
        }
    }
}

/** Common frame: insets, scrolling, a back arrow and a centred column of at most 560 dp. */
@Composable
private fun AuthFrame(onBack: (() -> Unit)?, content: @Composable ColumnScope.() -> Unit) {
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
private fun problemText(problem: SignInProblem): String = when (problem) {
    SignInProblem.MissingFields -> stringResource(R.string.signin_missing)
    SignInProblem.IncorrectPassword -> stringResource(R.string.signin_incorrect_password)
    SignInProblem.IncorrectCode -> stringResource(R.string.signin_incorrect_code)
    SignInProblem.PasswordsDiffer -> stringResource(R.string.recovery_mismatch)
    SignInProblem.PasswordTooShort -> stringResource(R.string.recovery_too_short, SignInViewModel.MIN_PASSWORD)
    is SignInProblem.Unreachable -> stringResource(R.string.connect_unreachable, problem.address)
    is SignInProblem.Other -> problem.message
}

@Composable
fun SignInScreen(vm: SignInViewModel, onChangeServer: () -> Unit, onRecovery: () -> Unit) {
    val state by vm.state.collectAsStateWithLifecycle()
    var password by remember { mutableStateOf("") } // secrets stay out of saved state
    var showSignupNote by rememberSaveable { mutableStateOf(false) }
    val submit = { vm.signIn(state.email, password) }
    AuthFrame(onBack = null) {
        ScreenHeader(
            title = stringResource(R.string.signin_title),
            subtitle = if (state.serverLabel.isNotEmpty()) stringResource(R.string.signin_subtitle, state.serverLabel) else null,
        )
        PvTextField(
            value = state.email,
            onValueChange = vm::onEmailChange,
            label = stringResource(R.string.signin_email),
            keyboardType = KeyboardType.Email,
            enabled = !state.busy,
        )
        PasswordField(
            value = password,
            onValueChange = { password = it; vm.clearProblem() },
            label = stringResource(R.string.signin_password),
            onIme = submit,
            isError = state.problem == SignInProblem.IncorrectPassword,
            enabled = !state.busy,
        )
        state.problem?.let { ProblemText(problemText(it)) }
        PrimaryButton(
            text = stringResource(R.string.signin_action),
            onClick = submit,
            loading = state.busy,
            modifier = Modifier.fillMaxWidth(),
        )
        LinkButton(stringResource(R.string.signin_use_recovery), onRecovery, Modifier.align(Alignment.CenterHorizontally))
        LinkButton(stringResource(R.string.signin_create_account), { showSignupNote = true }, Modifier.align(Alignment.CenterHorizontally))
        if (showSignupNote) {
            Text(
                stringResource(R.string.signin_create_account_note),
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.palette.muted,
            )
        }
        SecondaryButton(stringResource(R.string.signin_change_server), onChangeServer, Modifier.fillMaxWidth())
    }
}

@Composable
fun EmailCodeScreen(vm: SignInViewModel, onBack: () -> Unit) {
    val state by vm.state.collectAsStateWithLifecycle()
    var code by rememberSaveable { mutableStateOf("") }
    AuthFrame(onBack = onBack) {
        ScreenHeader(
            title = stringResource(R.string.code_email_title),
            subtitle = stringResource(R.string.code_email_subtitle, state.email),
            showBrand = false,
        )
        CodeField(code, { code = it.filter(Char::isDigit).take(6); vm.clearProblem() }, state.busy) { vm.verifyEmailCode(code) }
        state.problem?.let { ProblemText(problemText(it)) }
        PrimaryButton(stringResource(R.string.code_verify), { vm.verifyEmailCode(code) }, Modifier.fillMaxWidth(), loading = state.busy)
        LinkButton(stringResource(R.string.code_resend), vm::sendCode, Modifier.align(Alignment.CenterHorizontally), enabled = !state.busy)
    }
}

@Composable
fun TotpScreen(vm: SignInViewModel, sessionId: String, onBack: () -> Unit) {
    val state by vm.state.collectAsStateWithLifecycle()
    var code by rememberSaveable { mutableStateOf("") }
    AuthFrame(onBack = onBack) {
        ScreenHeader(
            title = stringResource(R.string.code_totp_title),
            subtitle = stringResource(R.string.code_totp_subtitle),
            showBrand = false,
        )
        CodeField(code, { code = it.filter(Char::isDigit).take(6); vm.clearProblem() }, state.busy) { vm.verifyTotp(sessionId, code) }
        state.problem?.let { ProblemText(problemText(it)) }
        PrimaryButton(stringResource(R.string.code_verify), { vm.verifyTotp(sessionId, code) }, Modifier.fillMaxWidth(), loading = state.busy)
    }
}

@Composable
private fun CodeField(code: String, onChange: (String) -> Unit, busy: Boolean, onDone: () -> Unit) {
    PvTextField(
        value = code,
        onValueChange = onChange,
        label = stringResource(R.string.code_label),
        keyboardType = KeyboardType.NumberPassword,
        imeAction = ImeAction.Done,
        onIme = onDone,
        enabled = !busy,
    )
}

@Composable
fun PasskeyScreen(url: String, onBack: () -> Unit) {
    val context = LocalContext.current
    var failed by rememberSaveable { mutableStateOf(false) }
    AuthFrame(onBack = onBack) {
        ScreenHeader(
            title = stringResource(R.string.passkey_title),
            subtitle = stringResource(R.string.passkey_subtitle),
            showBrand = false,
        )
        if (failed) ProblemText(stringResource(R.string.passkey_no_browser))
        PrimaryButton(
            text = stringResource(R.string.passkey_open),
            onClick = {
                try {
                    context.startActivity(Intent(Intent.ACTION_VIEW, Uri.parse(url)))
                } catch (e: ActivityNotFoundException) {
                    failed = true
                }
            },
            modifier = Modifier.fillMaxWidth(),
        )
    }
}

@Composable
fun RecoveryScreen(vm: SignInViewModel, onBack: () -> Unit) {
    val state by vm.state.collectAsStateWithLifecycle()
    var key by remember { mutableStateOf("") } // secrets stay out of saved state
    var password by remember { mutableStateOf("") } // secrets stay out of saved state
    var confirm by remember { mutableStateOf("") } // secrets stay out of saved state
    val submit = { vm.recover(key, password, confirm) }
    AuthFrame(onBack = onBack) {
        ScreenHeader(
            title = stringResource(R.string.recovery_title),
            subtitle = stringResource(R.string.recovery_subtitle),
            showBrand = false,
        )
        PvTextField(
            value = key,
            onValueChange = { key = it; vm.clearProblem() },
            label = stringResource(R.string.recovery_key),
            placeholder = stringResource(R.string.recovery_key_hint),
            singleLine = false,
            minLines = 3,
            enabled = !state.busy,
        )
        PasswordField(
            value = password,
            onValueChange = { password = it; vm.clearProblem() },
            label = stringResource(R.string.recovery_new_password),
            imeAction = ImeAction.Next,
            enabled = !state.busy,
        )
        PasswordField(
            value = confirm,
            onValueChange = { confirm = it; vm.clearProblem() },
            label = stringResource(R.string.recovery_confirm_password),
            onIme = submit,
            enabled = !state.busy,
        )
        state.problem?.let { ProblemText(problemText(it)) }
        PrimaryButton(stringResource(R.string.recovery_action), submit, Modifier.fillMaxWidth(), loading = state.busy)
    }
}
