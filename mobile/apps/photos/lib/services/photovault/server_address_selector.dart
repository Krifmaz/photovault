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
  ///
  /// [reachedAt] is an address the person actually connected through (typed
  /// LAN address, mDNS answer) when it is neither the home nor the public
  /// URL. It is used only when the public URL does not answer within
  /// [timeout] and [reachedAt] still answers with the same public URL;
  /// without it the public URL is used unchecked, as the contract says.
  Future<String> select(
    PhotovaultServerInfo server, {
    String? reachedAt,
  }) async {
    final public = server.publicUrl;
    final home = server.homeUrl;
    final hasHome =
        server.isPhotovault &&
        home != null &&
        home.isNotEmpty &&
        !sameServerAddress(home, public);
    if (hasHome && await _answers(home, server)) return home;
    if (!server.isPhotovault ||
        reachedAt == null ||
        reachedAt.isEmpty ||
        sameServerAddress(reachedAt, public) ||
        (hasHome && sameServerAddress(reachedAt, home))) {
      return public;
    }
    if (await _answers(public, server)) return public;
    if (await _answers(reachedAt, server)) return reachedAt;
    return public;
  }

  /// True when [baseUrl] answers `/photovault/info` within [timeout] as the
  /// same server (same public URL).
  Future<bool> _answers(String baseUrl, PhotovaultServerInfo server) async {
    try {
      final answer = await probe(baseUrl, timeout).timeout(timeout);
      return answer != null &&
          answer.isPhotovault &&
          sameServerAddress(answer.publicUrl, server.publicUrl);
    } catch (_) {
      return false;
    }
  }
}
