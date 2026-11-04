package app.photovault.android.backup

import app.photovault.android.data.CoreRepository

/**
 * What camera backup needs from photovault-core beyond [CoreRepository]'s upload queue
 * (contracts/photovault-core.md, "v1.2: backup"). The real core implements it in src/realCore (RealCore);
 * [FakeBackupCore] stands in for the demo library and the tests.
 */
interface BackupCore {
    /**
     * The id of the album the device folder [folderName] backs up into: the user's own `folder` collection
     * of that name, created the way Ente's apps create it when missing. Null means "no per-folder album"
     * (uploads then go to Uncategorized).
     */
    suspend fun backupAlbum(folderName: String): Long?
}

/** Demo and test stand-in: a stable made-up id per folder name. */
class FakeBackupCore : BackupCore {
    private val ids = HashMap<String, Long>()
    override suspend fun backupAlbum(folderName: String): Long =
        synchronized(ids) { ids.getOrPut(folderName.trim()) { 900_000L + ids.size } }
}

/** The core's backup side: the real one when the repository is photovault-core, else the fake. */
fun CoreRepository.backupCore(): BackupCore = this as? BackupCore ?: FakeBackupCore()
