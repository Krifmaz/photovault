// PHOTOVAULT: server connection (new file).
//
// One line on Ente's landing page saying which server the account will be
// created on / signed in to, with a "Change server" link back to the
// connect screen. Flat, no shadow.

import "package:ente_components/ente_components.dart";
import "package:ente_components/theme/photovault/encore_tokens.dart";
import "package:flutter/material.dart";
import "package:photos/services/photovault/photovault_strings.dart";
import "package:photos/services/photovault/server_address.dart";
import "package:photos/services/photovault/server_contract.dart";

class PhotovaultServerChip extends StatelessWidget {
  const PhotovaultServerChip({
    super.key,
    required this.server,
    required this.onChange,
    this.textColor = encoreDarkText,
    this.mutedColor = encoreDarkMuted,
  });

  final PhotovaultServerInfo server;
  final VoidCallback onChange;
  final Color textColor;
  final Color mutedColor;

  @override
  Widget build(BuildContext context) {
    final name = server.isPhotovault
        ? server.name
        : PhotovaultStrings.compatibleServer;
    return Padding(
      padding: const EdgeInsets.symmetric(horizontal: 8),
      child: Row(
        children: [
          Icon(Icons.dns_outlined, size: 18, color: mutedColor),
          const SizedBox(width: Spacing.sm),
          Expanded(
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Text(
                  PhotovaultStrings.connectedTo(name),
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                  style: TextStyles.mini.copyWith(color: textColor),
                ),
                Text(
                  displayServerAddress(server.publicUrl),
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                  style: TextStyles.tiny.copyWith(color: mutedColor),
                ),
              ],
            ),
          ),
          TextButton(
            onPressed: onChange,
            style: TextButton.styleFrom(
              foregroundColor: textColor,
              shape: RoundedRectangleBorder(
                borderRadius: BorderRadius.circular(EncoreTokens.radiusControl),
              ),
            ),
            child: Text(
              PhotovaultStrings.changeServer,
              style: TextStyles.mini.copyWith(
                color: textColor,
                decoration: TextDecoration.underline,
                decorationColor: textColor,
              ),
            ),
          ),
        ],
      ),
    );
  }
}
