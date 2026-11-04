package app.photovault.android.ui.backup

import app.photovault.android.MainDispatcherRule
import app.photovault.android.backup.BackupPhase
import app.photovault.android.backup.BackupSettings
import app.photovault.android.backup.FakeMedia
import app.photovault.android.backup.MediaAccess
import app.photovault.android.backup.MemoryBackupSettingsStore
import app.photovault.android.backup.QueueCore
import app.photovault.android.backup.RecordingScheduler
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.launch
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test

@OptIn(ExperimentalCoroutinesApi::class)
class BackupViewModelTest {
    @get:Rule val main = MainDispatcherRule()

    private val core = QueueCore()
    private val store = MemoryBackupSettingsStore()
    private val media = FakeMedia()
    private val scheduler = RecordingScheduler()
    private val phase = MutableStateFlow<BackupPhase>(BackupPhase.Idle)

    private fun TestScope.vm(): BackupViewModel {
        val vm = BackupViewModel(core, store, media, scheduler, phase, io = main.dispatcher, pollMs = 1_000)
        backgroundScope.launch { vm.state.collect {} }
        runCurrent()
        return vm
    }

    private suspend fun signIn() {
        core.addServer("photos.example.com")
        core.login("me@example.com", "secret")
    }

    @Test
    fun cameraFolderSelectedByDefault() = runTest(main.dispatcher) {
        signIn()
        val vm = vm()
        val s = vm.state.value
        assertTrue(s.loaded)
        assertTrue(s.signedIn)
        assertEquals(MediaAccess.Full, s.access)
        assertEquals(listOf(true, false), s.folders.map { it.selected })
        assertFalse(s.settings.enabled)
    }

    @Test
    fun turningOnSchedulesWithNewConstraints() = runTest(main.dispatcher) {
        signIn()
        val vm = vm()
        vm.setEnabled(true)
        runCurrent()
        assertTrue(vm.state.value.settings.enabled)
        assertEquals(BackupSettings(enabled = true) to true, scheduler.applied.last())
        vm.setChargingOnly(true)
        runCurrent()
        assertEquals(true, scheduler.applied.last().second)
        assertTrue(scheduler.applied.last().first.chargingOnly)
        vm.setIncludeVideos(false)
        runCurrent()
        assertEquals(false, scheduler.applied.last().second)
        vm.backUpNow()
        runCurrent()
        assertEquals(1, scheduler.nowRuns)
    }

    @Test
    fun togglingFoldersStartsFromTheCameraDefault() = runTest(main.dispatcher) {
        signIn()
        val vm = vm()
        vm.toggleFolder("shots")
        runCurrent()
        assertEquals(setOf("cam", "shots"), store.settings.value.folders)
        vm.toggleFolder("cam")
        runCurrent()
        assertEquals(setOf("shots"), store.settings.value.folders)
        assertEquals(listOf(false, true), vm.state.value.folders.map { it.selected })
    }

    @Test
    fun permissionGrantTurnsBackupOn() = runTest(main.dispatcher) {
        signIn()
        media.access = MediaAccess.None
        val vm = vm()
        assertEquals(MediaAccess.None, vm.state.value.access)
        assertTrue(vm.state.value.folders.isEmpty())
        // refused: stays off
        vm.onPermissionsResult(enableAfter = true)
        runCurrent()
        assertFalse(store.settings.value.enabled)
        media.access = MediaAccess.Full
        vm.onPermissionsResult(enableAfter = true)
        runCurrent()
        assertTrue(store.settings.value.enabled)
        assertEquals(2, vm.state.value.folders.size)
    }

    @Test
    fun showsQueueAndPhase() = runTest(main.dispatcher) {
        signIn()
        core.pending = 3
        val vm = vm()
        assertEquals(3, vm.state.value.queue?.pending)
        phase.value = BackupPhase.Scanning
        runCurrent()
        assertEquals(BackupPhase.Scanning, vm.state.value.phase)
        core.pending = 1
        testScheduler.advanceTimeBy(1_001)
        runCurrent()
        assertEquals(1, vm.state.value.queue?.pending)
    }

    @Test
    fun signedOut() = runTest(main.dispatcher) {
        val vm = vm()
        assertFalse(vm.state.value.signedIn)
        assertEquals(null, vm.state.value.queue)
    }
}
