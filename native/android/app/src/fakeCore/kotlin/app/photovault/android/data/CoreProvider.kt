package app.photovault.android.data

import android.content.Context
import app.photovault.android.data.fake.FakeCore

/** Built without photovault-core bindings: the in-memory demo library. */
object CoreProvider {
    fun create(@Suppress("UNUSED_PARAMETER") context: Context): CoreRepository = FakeCore()
}
