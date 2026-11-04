import 'dart:async';

import 'package:ente_components/ente_components.dart';
import 'package:ente_strings/ente_strings.dart';
import 'package:flutter/material.dart';
import 'package:hugeicons/hugeicons.dart';

class AboutSettingsSection extends StatelessWidget {
  const AboutSettingsSection({super.key, this.onCheckForUpdates});

  final FutureOr<void> Function()? onCheckForUpdates;

  @override
  Widget build(BuildContext context) {
    final strings = context.strings;
    final items = <Widget>[
      // PHOTOVAULT: strip - removed the Ente company links (source code on
      // Ente's GitHub, blog, privacy policy and terms).
      if (onCheckForUpdates != null)
        SettingsItem(
          title: strings.checkForUpdates,
          icon: HugeIcons.strokeRoundedDownload04,
          showOnlyLoadingState: true,
          onTap: onCheckForUpdates,
        ),
    ];

    return Column(
      mainAxisSize: MainAxisSize.min,
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        for (var index = 0; index < items.length; index++) ...[
          if (index > 0) const SizedBox(height: Spacing.sm),
          items[index],
        ],
      ],
    );
  }
}
