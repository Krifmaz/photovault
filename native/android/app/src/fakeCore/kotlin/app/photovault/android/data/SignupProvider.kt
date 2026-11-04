package app.photovault.android.data

import android.content.Context
import app.photovault.android.data.fake.FakeCore
import app.photovault.android.data.fake.FakeSignupRepository

/** Built without photovault-core bindings: the demo sign-up on top of FakeCore. */
object SignupProvider {
    fun create(@Suppress("UNUSED_PARAMETER") context: Context, core: CoreRepository): SignupRepository =
        if (core is FakeCore) FakeSignupRepository(core) else UnavailableSignupRepository()
}
