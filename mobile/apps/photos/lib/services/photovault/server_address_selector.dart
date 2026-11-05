// PHOTOVAULT: server connection (new file).
//
// Home/public address switching (contract: `contracts/server-info.md`):
// if the server has a home (LAN) address and it answers `/photovault/info`
// within 1.5 s with the same public URL, use it; otherwise use the public
// address. Re-run on app start and when connectivity changes. While
// Tailscale is up, the last address that answered is tried directly too.
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
  ///
  /// [vpnActive] (Tailscale is up): after home, [lastAddress] (the last
  /// address that answered) is tried directly before the public URL, since
  /// mDNS and the home LAN may be out of reach on cellular while the VPN
  /// still reaches the server.
  Future<String> select(
    PhotovaultServerInfo server, {
    String? reachedAt,
    String? lastAddress,
    bool vpnActive = false,
  }) async {
    final public = server.publicUrl;
    final home = server.homeUrl;
    final hasHome =
        server.isPhotovault &&
        home != null &&
        home.isNotEmpty &&
        !sameServerAddress(home, public);
    if (hasHome && await _answers(home, server)) return home;
    if (!server.isPhotovault) return public;
    bool extra(String? a) =>
        a != null &&
        a.isNotEmpty &&
        !sameServerAddress(a, public) &&
        !(hasHome && sameServerAddress(a, home));
    final fallbacks = <String>[];
    void add(String? a) {
      if (extra(a) && !fallbacks.any((f) => sameServerAddress(f, a))) {
        fallbacks.add(a!);
      }
    }

    if (vpnActive) add(lastAddress);
    final direct = fallbacks.length;
    add(reachedAt);
    if (fallbacks.isEmpty) return public;
    for (final a in fallbacks.take(direct)) {
      if (await _answers(a, server)) return a;
    }
    if (await _answers(public, server)) return public;
    for (final a in fallbacks.skip(direct)) {
      if (await _answers(a, server)) return a;
    }
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
