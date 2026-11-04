package app.photovault.android.backup

import android.content.ContentValues
import android.content.Context
import android.database.sqlite.SQLiteDatabase
import android.database.sqlite.SQLiteOpenHelper

/** What the ledger knows about one device file for one account. */
data class LedgerEntry(val modified: Long, val size: Long, val done: Boolean)

/**
 * Which device files were handed to photovault-core's upload queue, per account (server + user). It holds
 * MediaStore ids and timestamps only: no names, paths, keys or content. Core's own queue (sealed, resumable)
 * does the uploading; this only stops a file from being queued twice.
 *
 * `queued` rows become `done` once core's queue has drained. Core wipes its queue on sign-out, so a run that
 * finds core's queue empty first forgets the rows still `queued` ([forgetQueued]) and queues them again
 * (core skips content the library already has, by hash).
 */
interface BackupLedger {
    fun entries(account: String): Map<String, LedgerEntry>
    fun markQueued(account: String, key: String, modified: Long, size: Long)
    fun markAllDone(account: String)
    fun forgetQueued(account: String)
}

class SqliteBackupLedger(context: Context) :
    SQLiteOpenHelper(context, "backup-ledger.db", null, 1), BackupLedger {

    override fun onCreate(db: SQLiteDatabase) {
        db.execSQL(
            "CREATE TABLE ledger (account TEXT NOT NULL, media_key TEXT NOT NULL, modified INTEGER NOT NULL, " +
                "size INTEGER NOT NULL, done INTEGER NOT NULL DEFAULT 0, PRIMARY KEY (account, media_key))",
        )
    }

    override fun onUpgrade(db: SQLiteDatabase, oldVersion: Int, newVersion: Int) = Unit

    override fun entries(account: String): Map<String, LedgerEntry> {
        val out = HashMap<String, LedgerEntry>()
        readableDatabase.query("ledger", arrayOf("media_key", "modified", "size", "done"), "account = ?", arrayOf(account), null, null, null)
            .use { c ->
                while (c.moveToNext()) out[c.getString(0)] = LedgerEntry(c.getLong(1), c.getLong(2), c.getInt(3) != 0)
            }
        return out
    }

    override fun markQueued(account: String, key: String, modified: Long, size: Long) {
        val v = ContentValues().apply {
            put("account", account)
            put("media_key", key)
            put("modified", modified)
            put("size", size)
            put("done", 0)
        }
        writableDatabase.insertWithOnConflict("ledger", null, v, SQLiteDatabase.CONFLICT_REPLACE)
    }

    override fun markAllDone(account: String) {
        writableDatabase.execSQL("UPDATE ledger SET done = 1 WHERE account = ? AND done = 0", arrayOf(account))
    }

    override fun forgetQueued(account: String) {
        writableDatabase.delete("ledger", "account = ? AND done = 0", arrayOf(account))
    }
}

/** In memory, for tests. */
class MemoryBackupLedger : BackupLedger {
    private val rows = HashMap<String, HashMap<String, LedgerEntry>>()
    override fun entries(account: String): Map<String, LedgerEntry> = synchronized(rows) { HashMap(rows[account].orEmpty()) }
    override fun markQueued(account: String, key: String, modified: Long, size: Long) {
        synchronized(rows) { rows.getOrPut(account) { HashMap() }[key] = LedgerEntry(modified, size, false) }
    }
    override fun markAllDone(account: String) {
        synchronized(rows) { rows[account]?.replaceAll { _, e -> e.copy(done = true) } }
    }
    override fun forgetQueued(account: String) {
        synchronized(rows) { rows[account]?.values?.removeAll { !it.done } }
    }
}
