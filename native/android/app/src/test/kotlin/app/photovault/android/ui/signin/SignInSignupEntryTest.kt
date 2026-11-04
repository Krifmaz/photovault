package app.photovault.android.ui.signin

import app.photovault.android.MainDispatcherRule
import app.photovault.android.data.fake.FakeCore
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.advanceUntilIdle
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Rule
import org.junit.Test

/** "Create account" on the sign-in screen follows the server's sign-up setting. */
@OptIn(ExperimentalCoroutinesApi::class)
class SignInSignupEntryTest {
    @get:Rule val main = MainDispatcherRule()

    private val core = FakeCore(latencyMs = 10, ioDispatcher = main.dispatcher)

    @Test
    fun openServerOffersCreateAccount() = runTest(main.dispatcher) {
        core.addServer("photos.example.com")
        val vm = SignInViewModel(core, io = main.dispatcher)
        advanceUntilIdle()
        assertEquals(true, vm.state.value.signupsAllowed)
    }

    @Test
    fun closedServerSaysSo() = runTest(main.dispatcher) {
        core.addServer("nosignup.example.com")
        val vm = SignInViewModel(core, io = main.dispatcher)
        advanceUntilIdle()
        assertEquals(false, vm.state.value.signupsAllowed)
    }
}
