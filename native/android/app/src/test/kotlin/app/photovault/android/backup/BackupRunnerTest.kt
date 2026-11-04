package app.photovault.android.backup

import app.photovault.android.data.CoreError
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class BackupRunnerTest {
    private val core = QueueCore()
    private val albums = FakeBackupCore()
    private val media = FakeMedia()
    private val ledger = MemoryBackupLedger()
    private val settings = MemoryBackupSettingsStore(BackupSettings(enabled = true))
    private val runner = BackupRunner(core, albums, media, ledger, settings, pollMs = 1, clock = { 42L })

    private suspend fun signIn() {
        core.addServer("photos.example.com")
        core.login("me@example.com", "secret")
    }

    @Test
    fun offOrSignedOutOrNoAccessDoesNothing() = runTest {
        settings.update { it.copy(enabled = false) }
        assertEquals(BackupOutcome.Disabled, runner.run().outcome)
        settings.update { it.copy(enabled = true) }
        assertEquals(BackupOutcome.NotSignedIn, runner.run().outcome)
        signIn()
        media.access = MediaAccess.None
        assertEquals(BackupOutcome.NoPermission, runner.run().outcome)
        assertTrue(core.enqueued.isEmpty())
    }

    @Test
    fun queuesCameraFolderByDefaultIntoItsAlbumNewestFirst() = runTest {
        signIn()
        media.add(1)
        media.add(2)
        media.add(3, folder = "shots", name = "Screenshots")
        val phases = ArrayList<BackupPhase>()
        val r = runner.run { phases += it }
        assertEquals(BackupOutcome.Done, r.outcome)
        assertEquals(2, r.queued)
        assertEquals(42L, r.atMs)
        assertEquals(listOf("/sdcard/DCIM/Camera/2.jpg", "/sdcard/DCIM/Camera/1.jpg"), core.enqueued.map { it.first })
        val camera = albums.backupAlbum("Camera")
        assertTrue(core.enqueued.all { it.second == camera })
        assertTrue(phases.first() is BackupPhase.Scanning)
        assertTrue(phases.any { it is BackupPhase.Uploading })
        assertEquals(BackupPhase.Idle, phases.last())
        // all done: nothing queued again
        val again = runner.run()
        assertEquals(0, again.queued)
        assertEquals(2, core.enqueued.size)
        assertEquals(1, core.runs)
    }

    @Test
    fun chosenFoldersAndVideoOption() = runTest {
        signIn()
        media.add(1)
        media.add(2, folder = "shots", name = "Screenshots")
        media.add(3, video = true)
        settings.update { it.copy(folders = setOf("shots"), includeVideos = false) }
        runner.run()
        assertEquals(listOf("/sdcard/DCIM/Screenshots/2.jpg"), core.enqueued.map { it.first })
        settings.update { it.copy(folders = setOf("cam"), includeVideos = true) }
        runner.run()
        assertEquals(3, core.enqueued.size)
        assertTrue(core.enqueued.drop(1).all { it.second == albums.backupAlbum("Camera") })
    }

    @Test
    fun editedFileIsQueuedAgain() = runTest {
        signIn()
        media.add(1, modified = 10)
        runner.run()
        media.items.clear()
        media.add(1, modified = 20)
        assertEquals(1, runner.run().queued)
        assertEquals(2, core.enqueued.size)
    }

    @Test
    fun unreadableFilesAreSkippedAndRetriedLater() = runTest {
        signIn()
        media.add(1)
        media.add(2)
        media.unreadable += "/sdcard/DCIM/Camera/1.jpg"
        core.unreadable += "/sdcard/DCIM/Camera/2.jpg"
        val r = runner.run()
        assertEquals(0, r.queued)
        assertEquals(2, r.skipped)
        media.unreadable.clear()
        core.unreadable.clear()
        assertEquals(2, runner.run().queued)
    }

    @Test
    fun networkStopIsRetryAndResumesWithoutRequeuing() = runTest {
        signIn()
        media.add(1)
        core.stopKeepsPending = true
        assertEquals(BackupOutcome.Retry, runner.run().outcome)
        assertEquals(1, core.pending)
        // the next run uploads what core kept, without queuing the file a second time
        core.stopKeepsPending = false
        val r = runner.run()
        assertEquals(BackupOutcome.Done, r.outcome)
        assertEquals(0, r.queued)
        assertEquals(1, core.enqueued.size)
        assertEquals(0, core.pending)
    }

    @Test
    fun errorsMapToOutcomes() = runTest {
        signIn()
        media.add(1)
        core.stopWith = CoreError.Unreachable("10.0.1.7:8095")
        assertEquals(BackupOutcome.Retry, runner.run().outcome)
        core.stopWith = CoreError.Server(503, "busy")
        assertEquals(BackupOutcome.Retry, runner.run().outcome)
        core.stopWith = CoreError.Unauthorized()
        assertEquals(BackupOutcome.NotSignedIn, runner.run().outcome)
        core.stopWith = CoreError.Crypto("bad key")
        val r = runner.run()
        assertEquals(BackupOutcome.Error, r.outcome)
        assertEquals("bad key", r.message)
    }

    @Test
    fun queueWipedBySignOutIsQueuedAgain() = runTest {
        signIn()
        media.add(1)
        core.stopKeepsPending = true
        runner.run()
        // sign-out wipes core's queue before the file was uploaded
        core.wipeQueue()
        core.stopKeepsPending = false
        val r = runner.run()
        assertEquals(1, r.queued)
        assertEquals(2, core.enqueued.size)
    }

    @Test
    fun ledgerIsPerAccount() = runTest {
        signIn()
        media.add(1)
        runner.run()
        core.logout()
        // the same user id on another server is another account
        core.addServer("vault.example.org")
        core.login("me@example.com", "secret")
        assertEquals(1, runner.run().queued)
    }

    @Test
    fun plannerKeepsUnchangedFiles() {
        val a = LocalMedia("k1", "/a", "f", "F", 5, 10, false)
        val b = LocalMedia("k2", "/b", "f", "F", 5, 10, false)
        val known = mapOf("k1" to LedgerEntry(5, 10, true), "k2" to LedgerEntry(5, 11, true))
        assertEquals(listOf(b), BackupPlanner.plan(listOf(a, b), known))
    }

    @Test
    fun cameraFolders() {
        assertTrue(isCameraFolder("Camera", "DCIM/Camera/"))
        assertTrue(isCameraFolder("Anything", "/storage/emulated/0/DCIM/Camera"))
        assertTrue(isCameraFolder("Camera", null))
        assertTrue(!isCameraFolder("Screenshots", "Pictures/Screenshots/"))
        assertTrue(!isCameraFolder("Camera", "Download/Camera/"))
    }

    @Test
    fun permissionsPerAndroidVersion() {
        assertEquals(
            listOf("android.permission.READ_EXTERNAL_STORAGE"),
            backupPermissions(28).toList(),
        )
        assertTrue("android.permission.ACCESS_MEDIA_LOCATION" in backupPermissions(30))
        assertTrue("android.permission.READ_MEDIA_VISUAL_USER_SELECTED" in backupPermissions(34))
        assertTrue("android.permission.READ_MEDIA_VISUAL_USER_SELECTED" !in backupPermissions(33))
        assertTrue("android.permission.READ_EXTERNAL_STORAGE" !in backupPermissions(33))
    }
}
