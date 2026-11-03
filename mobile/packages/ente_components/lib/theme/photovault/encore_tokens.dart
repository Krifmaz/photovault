// PHOTOVAULT: theme tokens (new file).
//
// The single place in the Flutter apps where Photovault's "Encore" colours,
// radii and font are defined. Every value mirrors `branding/encore-tokens.json`
// at the repository root; change that file first, then this one.
//
// Values marked "derived" are not in the JSON: they are hover/pressed/tint
// shades of the JSON tokens, needed because Ente's colour schemes have more
// steps than Encore's stylesheet defines.
//
// The colours are top-level `const`s (not fields of an object) so that the
// existing `const` colour schemes can reference them directly:
// - the shared component theme, `ColorTokens.forApp(ComponentApp.photos)`
//   (see `encore_color_tokens.dart`), and
// - the Photos app's legacy `EnteColorScheme` (`apps/photos/lib/theme/colors.dart`).

import 'package:flutter/material.dart';

/// Non-colour Encore tokens. See `branding/encore-tokens.json`.
class EncoreTokens {
  const EncoreTokens._();

  /// Brand name shown in the UI (`brand.name`).
  static const String brandName = 'Photovault';

  /// Upstream credit line (`brand.credit`).
  static const String credit = 'Based on Ente (ente.com)';

  /// UI font family (`font.family`). The apps already bundle Inter.
  static const String fontFamily = 'Inter';

  /// Card radius (`radius.card`).
  static const double radiusCard = 14;

  /// Control (button, input) radius (`radius.control`).
  static const double radiusControl = 10;

  /// Small radius (`radius.small`).
  static const double radiusSmall = 8;

  /// Hero / sheet radius (`radius.hero`).
  static const double radiusHero = 20;
}

// ---------------------------------------------------------------------------
// Dark palette (`dark`).
// ---------------------------------------------------------------------------

/// Page background (`dark.bg`).
const Color encoreDarkBg = Color(0xFF161617);

/// Raised background (`dark.bg2`).
const Color encoreDarkBg2 = Color(0xFF1C1C1E);

/// Card / panel surface (`dark.panel`).
const Color encoreDarkPanel = Color(0xFF222224);

/// Secondary panel surface (`dark.panel2`).
const Color encoreDarkPanel2 = Color(0xFF2A2A2D);

/// Hairline border (`dark.border`).
const Color encoreDarkBorder = Color(0xFF353538);

/// Pressed panel / border. Derived: border lightened ~6%.
const Color encoreDarkBorderPressed = Color(0xFF3F3F42);

/// Primary text (`dark.text`).
const Color encoreDarkText = Color(0xFFF0EAD6);

/// Secondary text (`dark.muted`).
const Color encoreDarkMuted = Color(0xFFAAA493);

/// Tertiary text (`dark.faint`).
const Color encoreDarkFaint = Color(0xFF77725F);

/// Disabled text. Derived: faint at ~45% over panel.
const Color encoreDarkDisabled = Color(0xFF4A4740);

/// Primary accent: buttons and highlights (`dark.accent`).
const Color encoreDarkAccent = Color(0xFFF0EAD6);

/// Second accent stop (`dark.accent2`); used for hover.
const Color encoreDarkAccent2 = Color(0xFFD6C49A);

/// Pressed accent. Derived: accent2 darkened ~15%.
const Color encoreDarkAccentDarker = Color(0xFFB6A783);

/// Text and icons on the accent (`dark.onAccent`).
const Color encoreDarkOnAccent = Color(0xFF16161F);

/// Accent-coloured outline. Derived: accent2 at ~35% over bg.
const Color encoreDarkAccentStroke = Color(0xFF5A5240);

/// Success (`dark.ok`).
const Color encoreDarkOk = Color(0xFF22C55E);

/// Warning / caution, amber (`dark.warn`).
const Color encoreDarkWarn = Color(0xFFF59E0B);

/// Warning-tinted surface. Derived: warn at ~10% over bg.
const Color encoreDarkWarnTint = Color(0xFF2E2416);

/// Error / destructive (`dark.err`).
const Color encoreDarkErr = Color(0xFFEF4444);

/// Error hover. Derived: err darkened ~10%.
const Color encoreDarkErrDark = Color(0xFFD73D3D);

/// Error pressed. Derived: err darkened ~20%.
const Color encoreDarkErrDarker = Color(0xFFBF3636);

/// Error-tinted surface. Derived: err at ~10% over bg.
const Color encoreDarkErrTint = Color(0xFF2E1D1E);

/// Informational (`dark.info`).
const Color encoreDarkInfo = Color(0xFF38BDF8);

// ---------------------------------------------------------------------------
// Light palette (`light`).
// ---------------------------------------------------------------------------

/// Page background (`light.bg`).
const Color encoreLightBg = Color(0xFFF6F6FB);

/// Raised background (`light.bg2`).
const Color encoreLightBg2 = Color(0xFFFFFFFF);

/// Card / panel surface (`light.panel`).
const Color encoreLightPanel = Color(0xFFFFFFFF);

/// Secondary panel surface (`light.panel2`).
const Color encoreLightPanel2 = Color(0xFFF0F0F7);

/// Hairline border (`light.border`).
const Color encoreLightBorder = Color(0xFFE2E2EE);

/// Pressed panel / border. Derived: border darkened ~6%.
const Color encoreLightBorderPressed = Color(0xFFD5D5E3);

/// Primary text (`light.text`).
const Color encoreLightText = Color(0xFF16161F);

/// Secondary text (`light.muted`).
const Color encoreLightMuted = Color(0xFF5D5D75);

/// Tertiary text (`light.faint`).
const Color encoreLightFaint = Color(0xFF8D8DA5);

/// Disabled text. Derived: faint at ~45% over panel.
const Color encoreLightDisabled = Color(0xFFD3D3DF);

/// Primary accent: buttons and highlights (`light.accent`).
const Color encoreLightAccent = Color(0xFF2A2A2D);

/// Second accent stop (`light.accent2`); used for hover.
const Color encoreLightAccent2 = Color(0xFF8A7A4E);

/// Pressed accent. Derived: accent2 darkened ~15%.
const Color encoreLightAccentDarker = Color(0xFF756842);

/// Text and icons on the accent (`light.onAccent`).
const Color encoreLightOnAccent = Color(0xFFF0EAD6);

/// Accent-coloured outline. Derived: accent2 at ~35% over bg.
const Color encoreLightAccentStroke = Color(0xFFD3CCB8);

/// Success (`light.ok`).
const Color encoreLightOk = Color(0xFF22C55E);

/// Warning / caution, amber (`light.warn`).
const Color encoreLightWarn = Color(0xFFF59E0B);

/// Warning-tinted surface. Derived: warn at ~10% over bg.
const Color encoreLightWarnTint = Color(0xFFFDF3E3);

/// Error / destructive (`light.err`).
const Color encoreLightErr = Color(0xFFEF4444);

/// Error hover. Derived: err darkened ~10%.
const Color encoreLightErrDark = Color(0xFFD73D3D);

/// Error pressed. Derived: err darkened ~20%.
const Color encoreLightErrDarker = Color(0xFFBF3636);

/// Error-tinted surface. Derived: err at ~10% over bg.
const Color encoreLightErrTint = Color(0xFFFCEAEC);

/// Informational (`light.info`).
const Color encoreLightInfo = Color(0xFF38BDF8);
