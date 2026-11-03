package app.photovault.android.ui.signin

import app.photovault.android.MainDispatcherRule
import app.photovault.android.data.fake.FakeCore
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.test.advanceUntilIdle
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test

@OptIn(ExperimentalCoroutinesApi::class)
class SignInViewModelTest {
    @get:Rule val main = MainDispatcherRule()

    private val core = FakeCore(latencyMs = 50, ioDispatcher = main.dispatcher)

    private suspend fun vm(): SignInViewModel {
        core.addServer("photos.example.com")
        return SignInViewModel(core, io = main.dispatcher)
    }

    @Test
    fun showsTheActiveServer() = runTest(main.dispatcher) {
        val vm = vm()
        advanceUntilIdle()
        assertEquals("Demo vault", vm.state.value.serverLabel)
    }

    @Test
    fun passwordSignInFinishes() = runTest(main.dispatcher) {
        val vm = vm()
        vm.signIn(" me@example.com ", "secret")
        assertTrue(vm.state.value.busy)
        advanceUntilIdle()
        assertEquals(SignInEvent.Done, vm.eventFlow.first())
        assertFalse(vm.state.value.busy)
        assertEquals("me@example.com", core.currentAccount()?.email)
    }

    @Test
    fun wrongPasswordIsReported() = runTest(main.dispatcher) {
        val vm = vm()
        vm.signIn("me@example.com", "wrong")
        advanceUntilIdle()
        assertEquals(SignInProblem.IncorrectPassword, vm.state.value.problem)
        assertNull(core.currentAccount())
    }

    @Test
    fun missingFields() = runTest(main.dispatcher) {
        val vm = vm()
        vm.signIn("", "x")
        assertEquals(SignInProblem.MissingFields, vm.state.value.problem)
    }

    @Test
    fun emailCodeStep() = runTest(main.dispatcher) {
        val vm = vm()
        vm.signIn("me+otp@example.com", "secret")
        advanceUntilIdle()
        assertEquals(SignInEvent.EmailCode, vm.eventFlow.first())
        assertTrue(vm.state.value.codeSent)
        vm.verifyEmailCode("000000")
        advanceUntilIdle()
        assertEquals(SignInProblem.IncorrectCode, vm.state.value.problem)
        vm.verifyEmailCode(FakeCore.DEMO_CODE)
        advanceUntilIdle()
        assertEquals(SignInEvent.Done, vm.eventFlow.first())
    }

    @Test
    fun totpStep() = runTest(main.dispatcher) {
        val vm = vm()
        vm.signIn("me+totp@example.com", "secret")
        advanceUntilIdle()
        val step = vm.eventFlow.first()
        assertTrue(step is SignInEvent.Totp)
        vm.verifyTotp((step as SignInEvent.Totp).sessionId, FakeCore.DEMO_CODE)
        advanceUntilIdle()
        assertEquals(SignInEvent.Done, vm.eventFlow.first())
        assertNotNull(core.currentAccount())
    }

    @Test
    fun passkeyAndRecoverySteps() = runTest(main.dispatcher) {
        val vm = vm()
        vm.signIn("me+passkey@example.com", "secret")
        advanceUntilIdle()
        assertTrue(vm.eventFlow.first() is SignInEvent.Passkey)
        vm.signIn("me+recovery@example.com", "secret")
        advanceUntilIdle()
        assertEquals(SignInEvent.RecoveryKey, vm.eventFlow.first())
    }

    @Test
    fun useRecoveryKeyVerifiesEmailFirst() = runTest(main.dispatcher) {
        val vm = vm()
        vm.startRecovery()
        assertEquals(SignInProblem.MissingFields, vm.state.value.problem)
        vm.onEmailChange("me@example.com")
        vm.startRecovery()
        advanceUntilIdle()
        assertEquals(SignInEvent.EmailCode, vm.eventFlow.first())
        assertTrue(vm.state.value.codeSent)
        vm.verifyEmailCode(FakeCore.DEMO_CODE)
        advanceUntilIdle()
        assertEquals(SignInEvent.RecoveryKey, vm.eventFlow.first())
    }

    @Test
    fun recoveryKeyChecksPasswords() = runTest(main.dispatcher) {
        val vm = vm()
        vm.recover("word ".repeat(24), "longenough", "different")
        assertEquals(SignInProblem.PasswordsDiffer, vm.state.value.problem)
        vm.recover("word ".repeat(24), "short", "short")
        assertEquals(SignInProblem.PasswordTooShort, vm.state.value.problem)
        vm.recover("not a key", "longenough", "longenough")
        advanceUntilIdle()
        assertTrue(vm.state.value.problem is SignInProblem.Other)
        vm.recover("word ".repeat(24), "longenough", "longenough")
        advanceUntilIdle()
        assertEquals(SignInEvent.Done, vm.eventFlow.first())
    }
}
