// PHOTOVAULT: branding (new file).
//
// Static Photovault glyph used wherever Ente showed its duck mascot (empty
// states, onboarding, permission and progress screens, Rive animations). It
// is tinted with a theme colour and never animates.

import "package:ente_components/ente_components.dart";
import "package:flutter/widgets.dart";
import "package:flutter_svg/flutter_svg.dart";

/// The Photovault glyph, drawn inside a box of [width] x [height] (contained,
/// centred) in [color] (default: the theme's primary colour).
class PhotovaultGlyph extends StatelessWidget {
  const PhotovaultGlyph({
    super.key,
    this.width,
    this.height,
    this.color,
    this.scale = 0.6,
  });

  /// Asset path of the single-colour glyph.
  static const String asset = "assets/photovault_glyph.svg";

  final double? width;
  final double? height;
  final Color? color;

  /// Size of the glyph relative to the box, so it reads as an icon rather
  /// than filling the space the larger mascot artwork used.
  final double scale;

  @override
  Widget build(BuildContext context) {
    final tint = color ?? context.componentColors.primary;
    return SizedBox(
      width: width,
      height: height,
      child: FractionallySizedBox(
        widthFactor: scale,
        heightFactor: scale,
        child: SvgPicture.asset(
          asset,
          fit: BoxFit.contain,
          colorFilter: ColorFilter.mode(tint, BlendMode.srcIn),
        ),
      ),
    );
  }
}
