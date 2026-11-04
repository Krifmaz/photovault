package app.photovault.android.ui.nav

import androidx.compose.animation.AnimatedVisibilityScope
import androidx.compose.animation.ExperimentalSharedTransitionApi
import androidx.compose.animation.SharedTransitionScope
import androidx.compose.runtime.Composable
import androidx.compose.runtime.compositionLocalOf
import androidx.compose.ui.Modifier

/** The SharedTransitionLayout around the NavHost (null in previews and tests). */
@OptIn(ExperimentalSharedTransitionApi::class)
val LocalSharedTransitionScope = compositionLocalOf<SharedTransitionScope?> { null }

/** The current destination's AnimatedContentScope, for shared elements across screens. */
val LocalNavAnimatedScope = compositionLocalOf<AnimatedVisibilityScope?> { null }

/** Marks a photo as the same element in the grid and the viewer, so opening one morphs its bounds. */
@OptIn(ExperimentalSharedTransitionApi::class)
@Composable
fun Modifier.sharedPhoto(fileId: Long): Modifier {
    val shared = LocalSharedTransitionScope.current ?: return this
    val animated = LocalNavAnimatedScope.current ?: return this
    return with(shared) {
        this@sharedPhoto.sharedElement(rememberSharedContentState(key = "photo-$fileId"), animated)
    }
}
