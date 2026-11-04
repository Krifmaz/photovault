// PHOTOVAULT: server connection (new file).
//
// Scans the server's connect QR code with Ente's own live scanner
// (`packages/ente_qr_scanner`: CameraX + zxing-cpp on Android, AVFoundation
// on iOS; no Google Play services). Pops with the first code that reads as
// a server address; parsing is left to the caller.

import "dart:async";

import "package:ente_components/ente_components.dart";
import "package:ente_qr_scanner/ente_qr_scanner.dart";
import "package:flutter/material.dart";
import "package:photos/services/photovault/photovault_strings.dart";
import "package:photos/services/photovault/server_address.dart";
import "package:photos/services/photovault/server_contract.dart";

class ServerQrScanPage extends StatefulWidget {
  const ServerQrScanPage({super.key});

  @override
  State<ServerQrScanPage> createState() => _ServerQrScanPageState();
}

class _ServerQrScanPageState extends State<ServerQrScanPage> {
  StreamSubscription<String>? _subscription;
  bool _done = false;
  String? _message;

  void _onScannerCreated(EnteQrScannerController controller) {
    _subscription = controller.codes.listen((code) {
      if (_done) return;
      final payload = parseConnectPayload(code);
      if (payload == null || tryNormalizeServerAddress(payload.url) == null) {
        if (mounted) {
          setState(() => _message = PhotovaultStrings.qrNotAServer);
        }
        return;
      }
      if (!mounted) return;
      _done = true;
      Navigator.of(context).pop(code);
    });
  }

  @override
  void dispose() {
    unawaited(_subscription?.cancel());
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final colors = context.componentColors;
    return Scaffold(
      backgroundColor: colors.backgroundBase,
      appBar: AppBar(
        backgroundColor: colors.backgroundBase,
        elevation: 0,
        scrolledUnderElevation: 0,
        title: Text(
          PhotovaultStrings.scanQr,
          style: TextStyles.large.copyWith(color: colors.textBase),
        ),
      ),
      body: Column(
        children: [
          Expanded(
            child: EnteQrScannerView(
              overlay: EnteQrScannerOverlay(
                borderColor: colors.primary,
                overlayColor: Colors.black.withValues(alpha: 0.5),
                borderRadius: 14,
              ),
              onScannerCreated: _onScannerCreated,
              onError: (error) {
                if (mounted) setState(() => _message = error);
              },
            ),
          ),
          SafeArea(
            top: false,
            child: Padding(
              padding: const EdgeInsets.all(Spacing.lg),
              child: Text(
                _message ?? PhotovaultStrings.scanQrHint,
                textAlign: TextAlign.center,
                style: TextStyles.body.copyWith(
                  color: _message == null ? colors.textLight : colors.warning,
                ),
              ),
            ),
          ),
        ],
      ),
    );
  }
}
