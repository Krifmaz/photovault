package app.photovault.android.ui.connect

import app.photovault.android.MainDispatcherRule
import app.photovault.android.data.DiscoveredServer
import app.photovault.android.data.LanDiscovery
import app.photovault.android.data.TailscaleStatus
import app.photovault.android.data.fake.FakeCore
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.flow.flowOf
import kotlinx.coroutines.test.advanceUntilIdle
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test

@OptIn(ExperimentalCoroutinesApi::class)
class ConnectViewModelTest {
    @get:Rule val main = MainDispatcherRule()

    private val lanServer = DiscoveredServer("vault._photovault._tcp", "Home vault", "http://192.168.1.10:8080", "0.2.0")
    private val core = FakeCore(latencyMs = 50, ioDispatcher = main.dispatcher)
    private fun vm(default: String = "", tailscaleOn: Boolean = false) =
        ConnectViewModel(
            core,
            LanDiscovery { flowOf(listOf(lanServer)) },
            default,
            tailscaleStatus = TailscaleStatus { flowOf(tailscaleOn) },
            io = main.dispatcher,
        )

    @Test
    fun prefillsTheDefaultServer() {
        assertEquals("https://photos.example.com", vm("https://photos.example.com").state.value.address)
    }

    @Test
    fun checkShowsServerNameAndVersion() = runTest(main.dispatcher) {
        val vm = vm()
        vm.onAddressChange("photos.example.com")
        vm.check()
        assertTrue(vm.state.value.checking)
        advanceUntilIdle()
        val s = vm.state.value
        assertEquals("Demo vault", s.info?.name)
        assertEquals("0.2.0", s.info?.version)
        assertNull(s.problem)
    }

    @Test
    fun unreachableAddressExplainsItself() = runTest(main.dispatcher) {
        val vm = vm()
        vm.onAddressChange("unreachable.lan:8080")
        vm.check()
        advanceUntilIdle()
        assertEquals(ConnectProblem.Unreachable("unreachable.lan:8080"), vm.state.value.problem)
        assertNull(vm.state.value.info)
    }

    @Test
    fun emptyAddressIsAProblem() {
        val vm = vm()
        vm.check()
        assertEquals(ConnectProblem.EmptyAddress, vm.state.value.problem)
    }

    @Test
    fun typingClearsTheLastResult() = runTest(main.dispatcher) {
        val vm = vm()
        vm.onAddressChange("unreachable.lan")
        vm.check()
        advanceUntilIdle()
        vm.onAddressChange("photos.example.com")
        assertNull(vm.state.value.problem)
    }

    @Test
    fun connectSavesSelectsAndGoesToSignIn() = runTest(main.dispatcher) {
        val vm = vm()
        vm.onAddressChange("photos.example.com")
        vm.connect()
        advanceUntilIdle()
        assertEquals(ConnectEvent.SignIn, vm.eventFlow.first())
        assertEquals(listOf("http://photos.example.com"), vm.state.value.saved.map { it.publicUrl })
        assertEquals("http://photos.example.com", core.activeBaseUrl())
    }

    @Test
    fun savedServerWithAccountGoesToTimeline() = runTest(main.dispatcher) {
        val info = core.addServer("photos.example.com")
        core.login("me@example.com", "secret")
        val vm = vm()
        advanceUntilIdle()
        vm.selectSaved(info)
        advanceUntilIdle()
        assertEquals(ConnectEvent.Timeline, vm.eventFlow.first())
    }

    @Test
    fun removeSavedServer() = runTest(main.dispatcher) {
        val info = core.addServer("photos.example.com")
        val vm = vm()
        advanceUntilIdle()
        assertEquals(1, vm.state.value.saved.size)
        vm.removeSaved(info)
        advanceUntilIdle()
        assertTrue(vm.state.value.saved.isEmpty())
    }

    @Test
    fun qrCodeFillsAndChecksTheAddress() = runTest(main.dispatcher) {
        val vm = vm()
        vm.onQrScanned("photovault://connect?url=https%3A%2F%2Fphotos.example.com&name=Home")
        advanceUntilIdle()
        assertEquals("https://photos.example.com", vm.state.value.address)
        assertEquals("Demo vault", vm.state.value.info?.name)
    }

    @Test
    fun foreignQrCodeIsRejected() = runTest(main.dispatcher) {
        val vm = vm()
        vm.onQrScanned("WIFI:S:home;T:WPA;P:secret;;")
        advanceUntilIdle()
        assertEquals(ConnectProblem.BadQr, vm.state.value.problem)
        assertEquals("", vm.state.value.address)
    }

    @Test
    fun lanServersAreListedAndUsable() = runTest(main.dispatcher) {
        val vm = vm()
        assertEquals(listOf(lanServer), vm.lan.first { it.isNotEmpty() })
        vm.useDiscovered(lanServer)
        advanceUntilIdle()
        assertEquals("http://192.168.1.10:8080", vm.state.value.address)
        assertEquals("http://192.168.1.10:8080", vm.state.value.info?.homeUrl)
    }

    @Test
    fun recentServersAreMostRecentFirst() = runTest(main.dispatcher) {
        val a = core.addServer("a.example.com")
        core.addServer("b.example.com")
        val vm = vm()
        advanceUntilIdle()
        assertEquals(listOf("http://b.example.com", "http://a.example.com"), vm.state.value.recent.map { it.info.publicUrl })
        assertTrue(vm.state.value.recent.all { it.lastConnectedUs > 0 })
        vm.selectSaved(a)
        advanceUntilIdle()
        assertEquals(listOf("http://a.example.com", "http://b.example.com"), vm.state.value.recent.map { it.info.publicUrl })
        assertEquals(vm.state.value.recent.map { it.info }, vm.state.value.saved)
    }

    @Test
    fun connectRemembersTheAddressThatAnswered() = runTest(main.dispatcher) {
        val vm = vm()
        vm.onAddressChange("photos.example.com")
        vm.connect()
        advanceUntilIdle()
        val recent = vm.state.value.recent.single()
        assertEquals("http://photos.example.com", recent.lastAddress)
        assertTrue(recent.lastConnectedUs > 0)
    }

    @Test
    fun tailscaleOffTriesNothing() = runTest(main.dispatcher) {
        core.addServer("photos.example.com")
        val vm = vm(tailscaleOn = false)
        val scan = vm.tailscale.first()
        assertFalse(scan.active)
        assertTrue(scan.found.isEmpty())
    }

    @Test
    fun tailscaleTriesTheLastAddressDirectly() = runTest(main.dispatcher) {
        val info = core.addServer("photos.example.com")
        core.markSeen(info.publicUrl, "http://100.101.102.103:8080")
        val vm = vm(tailscaleOn = true)
        val scan = vm.tailscale.first { it.active && !it.searching && it.found.isNotEmpty() }
        val reach = scan.found.single()
        assertEquals("http://100.101.102.103:8080", reach.address)
        assertEquals(info.publicUrl, reach.server.info.publicUrl)
        assertTrue(reach.direct)

        // Tapping it selects the server; its plain-http Tailscale address is used once allowed.
        vm.useTailscale(reach)
        advanceUntilIdle()
        assertEquals(ConnectEvent.SignIn, vm.eventFlow.first())
        assertEquals("http://photos.example.com", core.activeBaseUrl())
        core.setHomeUrlAllowed(info.publicUrl, true)
        assertEquals("http://100.101.102.103:8080", core.activeBaseUrl())
    }

    @Test
    fun tailscaleFallsBackToThePublicAddress() = runTest(main.dispatcher) {
        val info = core.addServer("photos.example.com")
        core.markSeen(info.publicUrl, "http://unreachable.example:8080")
        val vm = vm(tailscaleOn = true)
        val reach = vm.tailscale.first { it.active && !it.searching && it.found.isNotEmpty() }.found.single()
        assertEquals("http://photos.example.com", reach.address)
        assertFalse(reach.direct)
    }

    @Test
    fun tailscaleWithoutRecentServersFindsNothing() = runTest(main.dispatcher) {
        val vm = vm(tailscaleOn = true)
        val scan = vm.tailscale.first { it.active && !it.searching }
        assertTrue(scan.found.isEmpty())
    }
}
