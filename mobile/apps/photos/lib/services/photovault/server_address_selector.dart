// PHOTOVAULT: server connection (new file).
//
// Home/public address switching (contract: `contracts/server-info.md`):
// if the server has a home (LAN) address and it answers `/photovault/info`
// within 1.5 s with the same public URL, use it; otherwise use the public
// address. Re-run on app start and when connectivity changes.
// Pure Dart; unit-tested in `test/photovault/server_address_selector_test.dart`.

import "package:photos/services/photovault/server_address.dart";
import "package:photos/services/photovault/server_contract.dart";

/// Fetches `/photovault/info` from a base URL, or null if it does not answer
/// within the timeout.
typedef ServerInfoProbe =
    Future<PhotovaultServerInfo?> Function(String baseUrl, Duration timeout);

class ServerAddressSelector {
  const ServerAddressSelector({
    required this.probe,
    this.timeout = kHomeProbeTimeout,
  });

  final ServerInfoProbe probe;
  final Duration timeout;

  /// The base URL to use for [server] right now.
  Future<String> select(PhotovaultServerInfo server) async {
    final public = server.publicUrl;
    final home = server.homeUrl;
    if (!server.isPhotovault ||
        home == null ||
        home.isEmpty ||
        sameServerAddress(home, public)) {
      return public;
    }
    try {
      final answer = await probe(home, timeout).timeout(timeout);
      if (answer != null &&
          answer.isPhotovault &&
          sameServerAddress(answer.publicUrl, public)) {
        return home;
      }
    } catch (_) {
      // Fall through to the public address.
    }
    return public;
  }
}
