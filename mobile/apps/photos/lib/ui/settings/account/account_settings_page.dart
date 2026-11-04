import "dart:async";

import "package:ente_account_deletion/account_deletion.dart";
import "package:ente_components/ente_components.dart";
import "package:ente_crypto/ente_crypto.dart";
import "package:ente_lock_screen/local_authentication_service.dart";
import "package:ente_strings/ente_strings.dart";
import "package:flutter/material.dart";
import "package:hugeicons/hugeicons.dart";
import "package:photos/services/account/user_service.dart";
import "package:photos/ui/account/change_email_dialog.dart";
import "package:photos/ui/account/password_entry_page.dart";
import "package:photos/ui/account/recovery_key_page.dart";
import "package:photos/utils/dialog_util.dart";

class AccountSettingsPage extends StatelessWidget {
  const AccountSettingsPage({super.key});

  @override
  Widget build(BuildContext context) {
    final l10n = context.strings;

    return SettingsPageScaffold(
      title: l10n.account,
      children: [
        // PHOTOVAULT: payments removed ("Manage subscription").
        SettingsItem(
          title: l10n.changeEmail,
          icon: HugeIcons.strokeRoundedMail01,
          showOnlyLoadingState: true,
          onTap: () async => _onChangeEmailTapped(context),
        ),
        const SizedBox(height: 8),
        SettingsItem(
          title: l10n.changePassword,
          icon: HugeIcons.strokeRoundedLockPassword,
          showOnlyLoadingState: true,
          onTap: () async => _onChangePasswordTapped(context),
        ),
        const SizedBox(height: 8),
        SettingsItem(
          title: l10n.recoveryKey,
          icon: HugeIcons.strokeRoundedKey01,
          showOnlyLoadingState: true,
          onTap: () async => _onRecoveryKeyTapped(context),
        ),
        // PHOTOVAULT: removed "Export your data" (a link to Ente's help site).
        const SizedBox(height: 8),
        SettingsItem(
          title: l10n.deleteAccount,
          icon: HugeIcons.strokeRoundedDelete02,
          isDestructive: true,
          onTap: () async => _onDeleteAccountTapped(context),
        ),
      ],
    );
  }

  Future<void> _onChangeEmailTapped(BuildContext context) async {
    final hasAuthenticated = await LocalAuthenticationService.instance
        .requestLocalAuthentication(
          context,
          context.strings.authToChangeYourEmail,
        );
    if (hasAuthenticated) {
      if (!context.mounted) return;
      unawaited(showChangeEmailBottomSheet(context));
    }
  }

  Future<void> _onChangePasswordTapped(BuildContext context) async {
    final hasAuthenticated = await LocalAuthenticationService.instance
        .requestLocalAuthentication(
          context,
          context.strings.authToChangeYourPassword,
        );
    if (hasAuthenticated) {
      if (!context.mounted) return;
      unawaited(
        Navigator.of(context).push(
          MaterialPageRoute(
            builder: (BuildContext context) {
              return const PasswordEntryPage(mode: PasswordEntryMode.update);
            },
          ),
        ),
      );
    }
  }

  Future<void> _onRecoveryKeyTapped(BuildContext context) async {
    final hasAuthenticated = await LocalAuthenticationService.instance
        .requestLocalAuthentication(
          context,
          context.strings.authToViewYourRecoveryKey,
        );
    if (hasAuthenticated) {
      String recoveryKey;
      try {
        if (!context.mounted) return;
        recoveryKey = await _getOrCreateRecoveryKey(context);
      } catch (e) {
        if (!context.mounted) return;
        await showGenericErrorDialog(context: context, error: e);
        return;
      }
      if (!context.mounted) return;
      unawaited(
        Navigator.of(context).push(
          MaterialPageRoute(
            builder: (BuildContext context) {
              return RecoveryKeyPage(
                recoveryKey,
                context.strings.ok,
                onDone: () {},
              );
            },
          ),
        ),
      );
    }
  }

  Future<String> _getOrCreateRecoveryKey(BuildContext context) async {
    return CryptoUtil.bin2hex(
      await UserService.instance.getOrCreateRecoveryKey(context),
    );
  }

  Future<void> _onDeleteAccountTapped(BuildContext context) async {
    final hasAuthenticated = await LocalAuthenticationService.instance
        .requestLocalAuthentication(
          context,
          context.strings.authToInitiateAccountDeletion,
        );
    if (!context.mounted || !hasAuthenticated) {
      return;
    }
    final deleted = await Navigator.of(context).push<bool>(
      MaterialPageRoute(
        builder: (BuildContext context) {
          return const DeleteAccountPage();
        },
      ),
    );
    if (deleted == true && context.mounted) {
      Navigator.of(context).popUntil((route) => route.isFirst);
    }
  }
}
