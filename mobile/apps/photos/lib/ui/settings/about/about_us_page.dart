import "package:ente_components/ente_components.dart";
import "package:ente_components/theme/photovault/encore_tokens.dart";
import "package:ente_strings/ente_strings.dart";
import "package:flutter/material.dart";

// PHOTOVAULT: branding. Ente's About page (What's new / changelog, open
// source, blog, privacy, terms and "Check for updates", all Ente links or
// services) is replaced by the single plain-text credit line "Based on Ente"
// (no link, no logo).
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
      ],
    );
  }
}
