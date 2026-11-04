// PHOTOVAULT: tree-specific setup for before_after_test.dart (this file is
// the Photovault version; tree_env_upstream.dart is copied over it when the
// same test runs on the pristine Ente import, see
// scripts/photovault/render_before_after.sh).
//
// Photovault shows its connect screen until a server is active, so the
// screens that follow (Ente's landing page, Settings) need one.

import "dart:io";

import "package:http/testing.dart";
import "package:photos/core/network/endpoint_config.dart";
import "package:photos/services/photovault/photovault_server_service.dart";
import "package:photos/services/photovault/server_contract.dart";
import "package:photos/services/photovault/server_discovery.dart";
import "package:photos/services/photovault/server_info_client.dart";
import "package:shared_preferences/shared_preferences.dart";

const treeName = "photovault";

class _NoDiscovery extends PhotovaultServerDiscovery {
  const _NoDiscovery();

  @override
  bool get isSupported => true;

  @override
  Future<List<DiscoveredServer>> search({
    Duration timeout = const Duration(seconds: 4),
  }) async => const [];
}

Future<void> prepareTree(SharedPreferences prefs) async {
  final service = PhotovaultServerService.instance;
  service.init(prefs, EndpointConfig(prefs), watchNetwork: false);
  service.overrideForTesting(
    client: PhotovaultServerInfoClient(
      clientFactory: () =>
          MockClient((_) async => throw const SocketException("offline")),
    ),
    discovery: const _NoDiscovery(),
  );
  await service.activate(
    const PhotovaultServerInfo(
      name: "Family photos",
      version: "0.4.0",
      publicUrl: "https://photos.example.com",
      homeUrl: "http://10.0.1.7:8095",
    ),
  );
}
