package app.photovault.android.ui.signup

import app.photovault.android.MainDispatcherRule
import app.photovault.android.data.Account
import app.photovault.android.data.CoreError
import app.photovault.android.data.SignupRepository
import app.photovault.android.data.fake.FakeCore
import app.photovault.android.data.fake.FakeSignupRepository
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.test.advanceUntilIdle
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test

@OptIn(ExperimentalCoroutinesApi::class)
class SignupViewModelTest {
    @get:Rule val main = MainDispatcherRule()

    private val core = FakeCore(latencyMs = 50, ioDispatcher = main.dispatcher)
    private val signup = FakeSignupRepository(core, latencyMs = 50)

    private suspend fun vm(address: String = "photos.example.com", repo: SignupRepository = signup): SignupViewModel {
        core.addServer(address)
        return SignupViewModel(core, repo, io = main.dispatcher)
    }

    @Test
    fun fullSignupSignsIn() = runTest(main.dispatcher) {
        val vm = vm()
        advanceUntilIdle()
        assertEquals(true, vm.state.value.signupsAllowed)
        assertEquals("Demo vault", vm.state.value.serverLabel)

        vm.onEmailChange(" new@example.com ")
        vm.submitEmail()
        assertEquals(SignupEvent.Password, vm.eventFlow.first())
        assertEquals("new@example.com", vm.state.value.email)

        vm.submitPassword(STRONG, STRONG)
        advanceUntilIdle()
        assertEquals(SignupEvent.Code, vm.eventFlow.first())

        vm.verifyCode("000000")
        advanceUntilIdle()
        assertEquals(SignupProblem.IncorrectCode, vm.state.value.problem)

        vm.verifyCode(FakeCore.DEMO_CODE)
        advanceUntilIdle()
        assertEquals(SignupEvent.RecoveryKey, vm.eventFlow.first())
        assertEquals(24, vm.state.value.recoveryWords.size)
        assertFalse(vm.state.value.makingKeys)
        assertNull(core.currentAccount())

        // Not before "I saved my recovery key".
        vm.finish()
        assertEquals(SignupProblem.NotSaved, vm.state.value.problem)
        assertNull(core.currentAccount())

        vm.setSaved(true)
        vm.finish()
        advanceUntilIdle()
        assertEquals(SignupEvent.Done, vm.eventFlow.first())
        assertEquals("new@example.com", core.currentAccount()?.email)
        assertTrue(vm.state.value.recoveryWords.isEmpty())
    }

    @Test
    fun emailIsChecked() = runTest(main.dispatcher) {
        val vm = vm()
        vm.submitEmail()
        assertEquals(SignupProblem.MissingFields, vm.state.value.problem)
        vm.onEmailChange("not an email")
        vm.submitEmail()
        assertEquals(SignupProblem.InvalidEmail, vm.state.value.problem)
        vm.onEmailChange("me@home")
        assertNull(vm.state.value.problem)
        vm.submitEmail()
        assertEquals(SignupEvent.Password, vm.eventFlow.first())
    }

    @Test
    fun passwordRules() = runTest(main.dispatcher) {
        val vm = vm()
        vm.onEmailChange("new@example.com")
        vm.submitPassword("", "")
        assertEquals(SignupProblem.MissingFields, vm.state.value.problem)
        vm.submitPassword(STRONG, "$STRONG!")
        assertEquals(SignupProblem.PasswordsDiffer, vm.state.value.problem)
        vm.submitPassword("Ab1!", "Ab1!")
        assertEquals(SignupProblem.PasswordTooShort, vm.state.value.problem)
        vm.submitPassword("password", "password")
        assertEquals(SignupProblem.PasswordTooWeak, vm.state.value.problem)
        vm.submitPassword("abcdefgh", "abcdefgh")
        assertEquals(SignupProblem.PasswordTooWeak, vm.state.value.problem)
        assertFalse(vm.state.value.busy)
    }

    @Test
    fun takenEmailIsReported() = runTest(main.dispatcher) {
        val vm = vm()
        vm.onEmailChange("taken@example.com")
        vm.submitEmail()
        vm.eventFlow.first()
        vm.submitPassword(STRONG, STRONG)
        advanceUntilIdle()
        assertEquals(SignupProblem.AlreadyRegistered, vm.state.value.problem)
    }

    @Test
    fun closedServerHidesTheForm() = runTest(main.dispatcher) {
        val vm = vm("nosignup.example.com")
        advanceUntilIdle()
        assertEquals(false, vm.state.value.signupsAllowed)
        // If the app still tried, the server's refusal is reported the same way.
        vm.onEmailChange("new@example.com")
        vm.submitPassword(STRONG, STRONG)
        advanceUntilIdle()
        assertEquals(SignupProblem.SignupsClosed, vm.state.value.problem)
    }

    @Test
    fun lostPasswordRestarts() = runTest(main.dispatcher) {
        // A restored code screen after process death has no password: start over.
        val vm = vm()
        vm.verifyCode(FakeCore.DEMO_CODE)
        assertEquals(SignupEvent.Restart, vm.eventFlow.first())
    }

    @Test
    fun keyGenerationFailureIsShown() = runTest(main.dispatcher) {
        val failing = object : SignupRepository by signup {
            override suspend fun prepare(password: String): List<String> =
                throw CoreError.Crypto("this device does not have enough memory to make secure keys")
        }
        val vm = vm(repo = failing)
        vm.onEmailChange("new@example.com")
        vm.submitPassword(STRONG, STRONG)
        advanceUntilIdle()
        vm.eventFlow.first()
        vm.verifyCode(FakeCore.DEMO_CODE)
        advanceUntilIdle()
        val p = vm.state.value.problem
        assertTrue("$p", p is SignupProblem.Other && "memory" in p.message)
        assertFalse(vm.state.value.makingKeys)
        assertFalse(vm.state.value.busy)
    }

    @Test
    fun lostEmailRestarts() = runTest(main.dispatcher) {
        // A restored password or code screen after process death has no email: start over, ask core nothing.
        var sent = 0
        val counting = object : SignupRepository by signup {
            override suspend fun sendCode(email: String) {
                sent++
                signup.sendCode(email)
            }
        }
        val vm = vm(repo = counting)
        vm.submitPassword(STRONG, STRONG)
        assertEquals(SignupEvent.Restart, vm.eventFlow.first())
        vm.resendCode()
        assertEquals(SignupEvent.Restart, vm.eventFlow.first())
        advanceUntilIdle()
        assertEquals(0, sent)
        assertNull(vm.state.value.problem)
    }

    @Test
    fun retryAfterAKeyFailureDoesNotReuseTheCode() = runTest(main.dispatcher) {
        var verified = 0
        var prepares = 0
        val flaky = object : SignupRepository by signup {
            override suspend fun verifyCode(email: String, code: String) {
                verified++
                // Codes work once, as on the server.
                if (verified > 1) throw CoreError.IncorrectCode()
                signup.verifyCode(email, code)
            }

            override suspend fun prepare(password: String): List<String> {
                if (++prepares == 1) throw CoreError.Crypto("this device does not have enough memory to make secure keys")
                return signup.prepare(password)
            }
        }
        val vm = vm(repo = flaky)
        vm.onEmailChange("new@example.com")
        vm.submitPassword(STRONG, STRONG)
        advanceUntilIdle()
        vm.eventFlow.first()
        vm.verifyCode(FakeCore.DEMO_CODE)
        advanceUntilIdle()
        assertTrue(vm.state.value.problem is SignupProblem.Other)
        // Verify again: only the keys are made.
        vm.verifyCode(FakeCore.DEMO_CODE)
        advanceUntilIdle()
        assertEquals(SignupEvent.RecoveryKey, vm.eventFlow.first())
        assertEquals(1, verified)
        assertEquals(2, prepares)
        assertNull(vm.state.value.problem)
    }

    @Test
    fun finishFailureKeepsTheRecoveryKey() = runTest(main.dispatcher) {
        val failing = object : SignupRepository by signup {
            override suspend fun finish(): Account = throw CoreError.Network("connection reset")
        }
        val vm = vm(repo = failing)
        vm.onEmailChange("new@example.com")
        vm.submitPassword(STRONG, STRONG)
        advanceUntilIdle()
        vm.eventFlow.first()
        vm.verifyCode(FakeCore.DEMO_CODE)
        advanceUntilIdle()
        vm.eventFlow.first()
        vm.setSaved(true)
        vm.finish()
        advanceUntilIdle()
        assertTrue(vm.state.value.problem is SignupProblem.Other)
        assertEquals(24, vm.state.value.recoveryWords.size)
    }

    private companion object {
        const val STRONG = "Correct horse battery staple 7"
    }
}
