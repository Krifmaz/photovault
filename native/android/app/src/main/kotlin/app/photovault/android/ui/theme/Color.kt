package app.photovault.android.ui.theme

import androidx.compose.runtime.Immutable
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color

/*
 * Encore colours: branding/encore-tokens.json ("dark" and "light"), which are Krifmaz/Encore@5c0bdcb
 * web-rs/static/css/app.css (:root[data-theme=encore] with the logo accent, and :root[data-theme=light]
 * with the charcoal accent of clients/android/.../ui/theme/Theme.kt). Change the JSON first, then here.
 */

/** The web's CSS variables for one theme: --bg, --bg-2, --panel, --panel-2, --border, --text, ... */
@Immutable
data class EncorePalette(
    val bg: Color,
    val bg2: Color,
    val panel: Color,
    val panel2: Color,
    val border: Color,
    val text: Color,
    val muted: Color,
    val faint: Color,
    val accent: Color,
    val accent2: Color,
    val onAccent: Color,
    val ok: Color,
    val warn: Color,
    val err: Color,
    val info: Color,
    val dark: Boolean,
) {
    /** `--grad`: the primary button, accent into accent-2 at 135 degrees. */
    val gradient: Brush get() = Brush.linearGradient(listOf(accent, accent2))
}

val EncoreDark = EncorePalette(
    bg = Color(0xFF161617),
    bg2 = Color(0xFF1C1C1E),
    panel = Color(0xFF222224),
    panel2 = Color(0xFF2A2A2D),
    border = Color(0xFF353538),
    text = Color(0xFFF0EAD6),
    muted = Color(0xFFAAA493),
    faint = Color(0xFF77725F),
    accent = Color(0xFFF0EAD6),
    accent2 = Color(0xFFD6C49A),
    onAccent = Color(0xFF16161F),
    ok = Color(0xFF22C55E),
    warn = Color(0xFFF59E0B),
    err = Color(0xFFEF4444),
    info = Color(0xFF38BDF8),
    dark = true,
)

val EncoreLight = EncorePalette(
    bg = Color(0xFFF6F6FB),
    bg2 = Color(0xFFFFFFFF),
    panel = Color(0xFFFFFFFF),
    panel2 = Color(0xFFF0F0F7),
    border = Color(0xFFE2E2EE),
    text = Color(0xFF16161F),
    muted = Color(0xFF5D5D75),
    faint = Color(0xFF8D8DA5),
    accent = Color(0xFF2A2A2D),
    accent2 = Color(0xFF8A7A4E),
    onAccent = Color(0xFFF0EAD6),
    ok = Color(0xFF22C55E),
    warn = Color(0xFFF59E0B),
    err = Color(0xFFEF4444),
    info = Color(0xFF38BDF8),
    dark = false,
)
