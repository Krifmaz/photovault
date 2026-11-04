package app.photovault.android.ui.settings

import app.photovault.android.MainDispatcherRule
import app.photovault.android.data.CoreError
import app.photovault.android.data.ml.ChargingSetting
import app.photovault.android.data.ml.MlScheduler
import app.photovault.android.data.ml.MlWork
import app.photovault.android.ui.memories.MemoriesTestMl
import app.photovault.android.ui.memories.mlStatus
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.launch
import kotlinx.coroutines.test.advanceTimeBy
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test

@OptIn(ExperimentalCoroutinesApi::class)
class MlSettingsViewModelTest {
    @get:Rule val main = MainDispatcherRule()

    private class Recorder : MlScheduler {
        val calls = mutableListOf<String>()
        override fun afterSync() { calls += "afterSync" }
        override fun reschedule() { calls += "reschedule" }
        override fun cancel() { calls += "cancel" }
    }

    private class Charging(var value: Boolean = true) : ChargingSetting {
        override fun get(): Boolean = value
        override fun set(value: Boolean) { this.value = value }
    }

    private val ml = MemoriesTestMl(status = mlStatus(enabled = false))
    private val scheduler = Recorder()
    private val charging = Charging()

    private fun vm() = MlSettingsViewModel(ml, charging, scheduler, io = main.dispatcher, pollMs = 1_000)

    @Test
    fun switchingOnSchedulesAndOffCancels() = runTest(main.dispatcher) {
        val vm = vm()
        val collector = launch { vm.state.collect {} }
        runCurrent()
        assertEquals(false, vm.state.value.status?.enabled)
        vm.setEnabled(true)
        runCurrent()
        assertEquals(listOf(true), ml.enabledCalls)
        assertTrue(vm.state.value.status!!.enabled)
        assertEquals(listOf("reschedule"), scheduler.calls)
        vm.setEnabled(false)
        runCurrent()
        assertFalse(vm.state.value.status!!.enabled)
        assertEquals(listOf("reschedule", "cancel"), scheduler.calls)
        collector.cancel()
    }

    @Test
    fun aFailedSwitchShowsTheError() = runTest(main.dispatcher) {
        val vm = vm()
        val collector = launch { vm.state.collect {} }
        ml.failSetEnabled = CoreError.Network("offline")
        vm.setEnabled(true)
        runCurrent()
        assertEquals("offline", vm.state.value.error)
        assertFalse(vm.state.value.busy)
        assertTrue(scheduler.calls.isEmpty())
        collector.cancel()
    }

    @Test
    fun chargingSettingIsStoredAndReschedulesOnlyWhenOn() = runTest(main.dispatcher) {
        val vm = vm()
        val collector = launch { vm.state.collect {} }
        runCurrent()
        assertTrue(vm.state.value.onlyWhileCharging)
        vm.setOnlyWhileCharging(false)
        runCurrent()
        assertFalse(charging.value)
        assertFalse(vm.state.value.onlyWhileCharging)
        assertTrue(scheduler.calls.isEmpty()) // ML is off
        ml.status = ml.status.copy(enabled = true)
        vm.setOnlyWhileCharging(true)
        runCurrent()
        assertEquals(listOf("reschedule"), scheduler.calls)
        collector.cancel()
    }

    @Test
    fun statusIsPolledWhileShown() = runTest(main.dispatcher) {
        val vm = vm()
        val collector = launch { vm.state.collect {} }
        runCurrent()
        ml.status = mlStatus(enabled = true, filesTotal = 10, filesIndexed = 4)
        advanceTimeBy(1_001)
        runCurrent()
        assertEquals(4L, vm.state.value.status?.filesIndexed)
        collector.cancel()
    }

    @Test
    fun statusLines() {
        assertTrue(statusLines(null).isEmpty())
        assertTrue(statusLines(mlStatus(enabled = false, filesTotal = 5)).isEmpty())
        val downloading = statusLines(
            mlStatus(enabled = true, modelBytesDone = 50L * 1024 * 1024, modelBytesTotal = 200L * 1024 * 1024, filesTotal = 100, filesIndexed = 0, running = true),
        )
        assertEquals(
            listOf(MlStatusLine.DownloadingModels(50, 200), MlStatusLine.Indexed(0, 100), MlStatusLine.Running),
            downloading,
        )
        val noInference = statusLines(mlStatus(enabled = true, inferenceAvailable = false, filesTotal = 3, filesIndexed = 3, lastError = "boom"))
        assertEquals(listOf(MlStatusLine.NoInference, MlStatusLine.Indexed(3, 3), MlStatusLine.Problem("boom")), noInference)
    }

    @Test
    fun workConstraintsAndDecisions() {
        val first = MlWork.indexConstraints(onlyWhileCharging = true, modelsReady = false)
        assertTrue(first.requiresCharging && first.batteryNotLow && first.unmetered)
        val later = MlWork.indexConstraints(onlyWhileCharging = false, modelsReady = true)
        assertFalse(later.requiresCharging || later.unmetered)
        assertTrue(later.batteryNotLow)

        assertTrue(MlWork.shouldIndex(mlStatus(enabled = true, filesPending = 3)))
        assertFalse(MlWork.shouldIndex(mlStatus(enabled = false, filesPending = 3)))
        assertFalse(MlWork.shouldIndex(mlStatus(enabled = true, inferenceAvailable = false, filesPending = 3)))
        assertFalse(MlWork.shouldIndex(mlStatus(enabled = true, filesPending = 0)))

        assertTrue(MlWork.retryable(CoreError.Network("x")))
        assertTrue(MlWork.retryable(CoreError.Server(502, "bad gateway")))
        assertFalse(MlWork.retryable(CoreError.NotLoggedIn()))
    }
}
