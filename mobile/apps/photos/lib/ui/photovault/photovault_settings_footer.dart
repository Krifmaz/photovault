// PHOTOVAULT: branding (new file).
//
// Settings footer for Photovault. Replaces Ente's "More from ente" footer
// (Ente wordmark, links to Ente Auth/Locker and Ente's social accounts),
// keeping the same spacing and the tappable app version, and shows the
// upstream credit line required by the project.

import "package:ente_components/ente_components.dart";
import "package:ente_components/theme/photovault/encore_tokens.dart";
import "package:ente_ui/components/settings/app_version_widget.dart";
import "package:flutter/material.dart";

/// Footer at the bottom of the settings page: credit line and app version.
class PhotovaultSettingsFooter extends StatelessWidget {
  /// Creates the footer; [onVersionTap] runs when the version is tapped.
  const PhotovaultSettingsFooter({super.key, this.onVersionTap});

  /// Called when the app version line is tapped (opens the change log).
  final VoidCallback? onVersionTap;

  @override
  Widget build(BuildContext context) {
    return SizedBox(
      width: double.infinity,
      child: Column(
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          const SizedBox(height: 40),
          Text(
            EncoreTokens.credit,
            textAlign: TextAlign.center,
            style: TextStyles.mini.copyWith(
              color: context.componentColors.textLight,
            ),
          ),
          AppVersionWidget(onTap: onVersionTap),
          const SizedBox(height: Spacing.xxl),
        ],
      ),
    );
  }
}
