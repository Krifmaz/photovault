// PHOTOVAULT: server connection (new file).
//
// Settings > Server: the current server's name, version and addresses, and
// "Switch server". Switching while signed in first logs out with Ente's own
// logout (`UserService.logout`); the saved servers survive it and the
// connect screen then opens on the landing page.

import "package:ente_components/ente_components.dart";
import "package:ente_components/theme/photovault/encore_tokens.dart";
import "package:ente_pure_utils/ente_pure_utils.dart";
import "package:flutter/material.dart";
import "package:hugeicons/hugeicons.dart";
import "package:photos/core/configuration.dart";
import "package:photos/services/account/user_service.dart";
import "package:photos/services/photovault/photovault_server_service.dart";
import "package:photos/services/photovault/photovault_strings.dart";
import "package:photos/services/photovault/server_address.dart";
import "package:photos/services/photovault/server_contract.dart";
import "package:photos/ui/photovault/connect_server_page.dart";
import "package:photos/utils/dialog_util.dart";

class ServerSettingsPage extends StatelessWidget {
  const ServerSettingsPage({super.key});

  @override
  Widget build(BuildContext context) {
    final service = PhotovaultServerService.instance;
    return ValueListenableBuilder<PhotovaultServerInfo?>(
      valueListenable: service.activeServer,
      builder: (context, server, _) {
        final colors = context.componentColors;
        return SettingsPageScaffold(
          title: PhotovaultStrings.server,
          children: [
            Container(
              padding: const EdgeInsets.all(Spacing.lg),
              decoration: BoxDecoration(
                color: colors.fillLight,
                borderRadius: BorderRadius.circular(EncoreTokens.radiusCard),
                border: Border.all(color: colors.strokeFaint),
              ),
              child: server == null
                  ? Text(
                      PhotovaultStrings.notConnected,
                      style: TextStyles.body.copyWith(color: colors.textLight),
                    )
                  : _ServerDetails(
                      server: server,
                      current: service.currentAddress,
                    ),
            ),
            const SizedBox(height: Spacing.lg),
            SettingsItem(
              title: PhotovaultStrings.switchServer,
              icon: HugeIcons.strokeRoundedArrowDataTransferHorizontal,
              onTap: () => _switchServer(context, server),
            ),
          ],
        );
      },
    );
  }

  Future<void> _switchServer(
    BuildContext context,
    PhotovaultServerInfo? server,
  ) async {
    final service = PhotovaultServerService.instance;
    if (!Configuration.instance.isLoggedIn()) {
      // Offline gallery or not signed in: nothing to log out of.
      await routeToPage(context, const ConnectServerPage());
      return;
    }
    final name = server == null
        ? PhotovaultStrings.server
        : (server.isPhotovault
              ? server.name
              : displayServerAddress(server.publicUrl));
    await showChoiceActionSheet(
      context,
      title: PhotovaultStrings.switchServer,
      body: PhotovaultStrings.switchServerBody(name),
      firstButtonLabel: PhotovaultStrings.switchServer,
      isCritical: true,
      firstButtonOnTap: () async {
        await UserService.instance.logout(context);
        if (!Configuration.instance.isLoggedIn()) {
          // Back on the landing page, which shows the connect screen.
          await service.clearActive();
        }
      },
    );
  }
}

class _ServerDetails extends StatelessWidget {
  const _ServerDetails({required this.server, required this.current});

  final PhotovaultServerInfo server;
  final String current;

  @override
  Widget build(BuildContext context) {
    final colors = context.componentColors;
    final rows = <(String, String)>[
      if (server.version.isNotEmpty)
        (PhotovaultStrings.version, server.version),
      if (server.homeUrl != null)
        (PhotovaultStrings.homeAddress, _mark(server.homeUrl!, current)),
      (
        server.homeUrl != null
            ? PhotovaultStrings.publicAddress
            : PhotovaultStrings.address,
        _mark(server.publicUrl, current),
      ),
    ];
    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        Text(
          server.isPhotovault
              ? server.name
              : PhotovaultStrings.compatibleServer,
          style: TextStyles.large.copyWith(color: colors.textBase),
        ),
        const SizedBox(height: Spacing.sm),
        for (final (label, value) in rows)
          Padding(
            padding: const EdgeInsets.only(top: Spacing.xs),
            child: Row(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                SizedBox(
                  width: 120,
                  child: Text(
                    label,
                    style: TextStyles.mini.copyWith(color: colors.textLight),
                  ),
                ),
                Expanded(
                  child: SelectableText(
                    value,
                    style: TextStyles.mini.copyWith(color: colors.textBase),
                  ),
                ),
              ],
            ),
          ),
      ],
    );
  }

  static String _mark(String url, String current) {
    final shown = displayServerAddress(url);
    return sameServerAddress(url, current)
        ? "$shown (${PhotovaultStrings.inUse})"
        : shown;
  }
}
