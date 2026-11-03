// PHOTOVAULT: branding (new file).
//
// Settings footer for Photovault. Replaces Ente's "More from ente" footer
// (Ente wordmark, links to Ente Auth/Locker and Ente's social accounts),
// keeping the same spacing and the app version. The upstream credit line
// lives on the About page only.

import "package:ente_components/ente_components.dart";
import "package:ente_ui/components/settings/app_version_widget.dart";
import "package:flutter/material.dart";

/// Footer at the bottom of the settings page: the app version.
class PhotovaultSettingsFooter extends StatelessWidget {
  /// Creates the footer; [onVersionTap] runs when the version is tapped.
  const PhotovaultSettingsFooter({super.key, this.onVersionTap});

  /// Called when the app version line is tapped.
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
          AppVersionWidget(onTap: onVersionTap),
          const SizedBox(height: Spacing.xxl),
        ],
      ),
    );
  }
}
