import 'package:ente_components/ente_components.dart';
import 'package:flutter/material.dart';
import 'package:hugeicons/hugeicons.dart';
import 'package:url_launcher/url_launcher_string.dart';

class SocialIconsRow extends StatelessWidget {
  const SocialIconsRow({super.key});

  @override
  Widget build(BuildContext context) {
    return Wrap(
      alignment: WrapAlignment.center,
      spacing: Spacing.xs,
      runSpacing: Spacing.xs,
      children: _socialLinks
          .map(
            (link) => Semantics(
              label: link.label,
              button: true,
              child: IconButtonComponent(
                tooltip: link.label,
                shouldSurfaceExecutionStates: false,
                icon: HugeIcon(
                  icon: link.icon,
                  size: IconSizes.small,
                  strokeWidth: 1.6,
                ),
                onTap: () => launchUrlString(
                  link.url,
                  mode: LaunchMode.externalApplication,
                ),
              ),
            ),
          )
          .toList(growable: false),
    );
  }
}

// PHOTOVAULT: strip - Ente's social accounts (Ente's /discord page, YouTube,
// GitHub, X, Mastodon, Reddit) removed; the row renders nothing.
const _socialLinks = <_SocialLink>[];

class _SocialLink {
  const _SocialLink({
    required this.label,
    required this.icon,
    required this.url,
  });

  final String label;
  final List<List<dynamic>> icon;
  final String url;
}
