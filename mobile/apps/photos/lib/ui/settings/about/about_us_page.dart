import "package:ente_components/ente_components.dart";
import "package:ente_components/theme/photovault/encore_tokens.dart";
import "package:ente_strings/ente_strings.dart";
import "package:flutter/material.dart";
import "package:hugeicons/hugeicons.dart"; // PHOTOVAULT
import "package:photos/utils/email_util.dart"; // PHOTOVAULT

// PHOTOVAULT: branding. Ente's About page (What's new / changelog, open
// source, blog, privacy, terms and "Check for updates", all Ente links or
// services) is replaced by the single plain-text credit line "Based on Ente"
// (no link, no logo), plus a "Logs" entry (view/export only).
class AboutUsPage extends StatelessWidget {
  const AboutUsPage({super.key});

  @override
  Widget build(BuildContext context) {
    return SettingsPageScaffold(
      title: context.strings.about,
      children: [
        Padding(
          padding: const EdgeInsets.symmetric(vertical: Spacing.lg),
          child: Text(
            EncoreTokens.credit,
            textAlign: TextAlign.center,
            style: TextStyles.body.copyWith(
              color: context.componentColors.textLight,
            ),
          ),
        ),
        // PHOTOVAULT: logs. Help & Support (which held View/Export logs) is
        // removed with Ente's support links; this keeps the logs reachable
        // for self-hosters. sendLogs offers only View and Export.
        SettingsItem(
          title: context.strings.logs,
          icon: HugeIcons.strokeRoundedNote01,
          onTap: () => sendLogs(context, context.strings.logs, ""),
        ),
      ],
    );
  }
}
