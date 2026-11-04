package app.photovault.android.ui.theme

import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.ColorScheme
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Shapes
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.Immutable
import androidx.compose.runtime.ReadOnlyComposable
import androidx.compose.runtime.remember
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp

/** encore-tokens.json "radius": card 14, control 10, small 8, pill 99, hero 20, tile 12. */
object EncoreRadius {
    val card: Dp = 14.dp
    val control: Dp = 10.dp
    val small: Dp = 8.dp
    val pill: Dp = 99.dp
    val hero: Dp = 20.dp
    val tile: Dp = 12.dp
}

/** encore-tokens.json "spacing". */
@Immutable
object Spacing {
    val xxs: Dp = 2.dp
    val xs: Dp = 4.dp
    val sm: Dp = 6.dp
    val md: Dp = 8.dp
    val base: Dp = 10.dp
    val lg: Dp = 12.dp
    val xl: Dp = 16.dp
    val xxl: Dp = 20.dp
    val section: Dp = 28.dp
}

/** encore-tokens.json "motion". */
object EncoreMotion {
    const val FAST_MS: Int = 150
    const val NORMAL_MS: Int = 200
}

/** Encore's Material shapes: thumbs 8, buttons/inputs 10, tiles/menus 12, cards 14, hero 20. */
val EncoreShapes: Shapes = Shapes(
    extraSmall = RoundedCornerShape(EncoreRadius.small),
    small = RoundedCornerShape(EncoreRadius.control),
    medium = RoundedCornerShape(EncoreRadius.tile),
    large = RoundedCornerShape(EncoreRadius.card),
    extraLarge = RoundedCornerShape(EncoreRadius.hero),
)

val LocalEncorePalette = staticCompositionLocalOf { EncoreDark }

/** The current theme's Encore tokens (MaterialTheme.colorScheme carries the same colours for Material widgets). */
val MaterialTheme.palette: EncorePalette
    @Composable
    @ReadOnlyComposable
    get() = LocalEncorePalette.current

private fun schemeOf(p: EncorePalette): ColorScheme {
    val base = if (p.dark) darkColorScheme() else lightColorScheme()
    return base.copy(
        primary = p.accent,
        onPrimary = p.onAccent,
        primaryContainer = p.panel2,
        onPrimaryContainer = p.text,
        secondary = p.accent2,
        onSecondary = p.onAccent,
        secondaryContainer = p.panel2,
        onSecondaryContainer = p.text,
        tertiary = p.accent2,
        onTertiary = p.onAccent,
        background = p.bg,
        onBackground = p.text,
        surface = p.bg2,
        onSurface = p.text,
        surfaceVariant = p.panel,
        onSurfaceVariant = p.muted,
        surfaceContainerLowest = p.bg,
        surfaceContainerLow = p.bg2,
        surfaceContainer = p.panel,
        surfaceContainerHigh = p.panel2,
        surfaceContainerHighest = p.border,
        surfaceTint = Color.Transparent,
        outline = p.border,
        outlineVariant = p.faint,
        error = p.err,
        onError = Color.White,
        inverseSurface = p.text,
        inverseOnSurface = p.bg,
        scrim = Color.Black,
    )
}

/** Encore, dark (charcoal and cream) or light (white and charcoal) following the system setting. */
@Composable
fun PhotovaultTheme(dark: Boolean = isSystemInDarkTheme(), content: @Composable () -> Unit) {
    val palette = if (dark) EncoreDark else EncoreLight
    val scheme = remember(palette) { schemeOf(palette) }
    CompositionLocalProvider(LocalEncorePalette provides palette) {
        MaterialTheme(colorScheme = scheme, typography = EncoreTypography, shapes = EncoreShapes, content = content)
    }
}
