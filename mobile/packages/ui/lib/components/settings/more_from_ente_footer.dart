import 'package:ente_components/ente_components.dart';
import 'package:ente_ui/components/settings/app_version_widget.dart';
import 'package:flutter/material.dart';

class MoreFromEnteFooter extends StatelessWidget {
  const MoreFromEnteFooter({
    super.key,
    required this.currentApp,
    this.onVersionTap,
  });

  final ComponentApp currentApp;
  final VoidCallback? onVersionTap;

  @override
  Widget build(BuildContext context) {
    return SizedBox(
      width: double.infinity,
      child: Column(
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          // PHOTOVAULT: strip - removed "More from Ente" (links to Ente's website)
          // and Ente's social links; only the version stays.
          const SizedBox(height: 40),
          AppVersionWidget(onTap: onVersionTap),
          const SizedBox(height: Spacing.xxl),
        ],
      ),
    );
  }
}
