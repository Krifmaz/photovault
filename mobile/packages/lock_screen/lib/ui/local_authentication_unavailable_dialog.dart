import 'package:ente_components/ente_components.dart';
import 'package:ente_lock_screen/auth_util.dart';
import 'package:ente_strings/ente_strings.dart';
import 'package:ente_ui/utils/toast_util.dart';
import 'package:flutter/material.dart';

bool shouldShowLinuxSystemAuthSetupGuide(
  LocalAuthenticationUnavailableException exception,
) {
  return exception.issue == LocalAuthUnavailableIssue.linuxSetupRequired;
}

Future<void> showLocalAuthenticationUnavailableMessage(
  BuildContext context,
  LocalAuthenticationUnavailableException exception, {
  VoidCallback? onOpenGuide,
}) async {
  if (shouldShowLinuxSystemAuthSetupGuide(exception)) {
    await showLinuxSystemAuthSetupDialog(context, onOpenGuide: onOpenGuide);
    return;
  }
  showToast(context, exception.userMessage);
}

// PHOTOVAULT: strip - the "Open guide" button (a help page on Ente's website,
// Ente's /help/auth/troubleshooting/linux-system-auth page) is removed and the
// text no longer names Ente Auth. `onOpenGuide` is kept for callers.
Future<void> showLinuxSystemAuthSetupDialog(
  BuildContext context, {
  VoidCallback? onOpenGuide,
}) async {
  await showBottomSheetComponent(
    context: context,
    builder: (context) => BottomSheetComponent(
      title: pendingTranslation('Linux setup required'),
      message: pendingTranslation(
        'To use device lock on Linux, this app needs a one-time system authentication (polkit) setup. See your distribution\'s documentation; Flatpak and AppImage installs and fingerprint prompts may need extra steps.',
      ),
      isScrollable: true,
    ),
  );
}
