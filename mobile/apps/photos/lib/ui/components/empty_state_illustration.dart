import "package:flutter/material.dart";
// PHOTOVAULT: branding. Ente played a Rive duck animation from [assetPath];
// Photovault shows its static glyph in the same box instead.
import "package:photos/ui/photovault/photovault_glyph.dart";

class EmptyStateIllustration extends StatelessWidget {
  const EmptyStateIllustration({
    required this.assetPath,
    required this.size,
    super.key,
  });

  /// Kept for upstream call sites; no longer loaded (PHOTOVAULT).
  final String assetPath;
  final Size size;

  @override
  Widget build(BuildContext context) {
    return PhotovaultGlyph(width: size.width, height: size.height);
  }
}
