package app.photovault.android.data

import android.content.Context
import app.photovault.android.data.fake.FakeCore
import app.photovault.android.data.ml.MlRepository
import app.photovault.android.data.ml.fake.FakeMlRepository

/** Built without photovault-core bindings: the in-memory demo library. */
object CoreProvider {
    fun create(@Suppress("UNUSED_PARAMETER") context: Context): CoreRepository = FakeCore()

    /** Search, people and memories over the demo library [core] serves. */
    fun createMl(core: CoreRepository): MlRepository = FakeMlRepository(core)
}
