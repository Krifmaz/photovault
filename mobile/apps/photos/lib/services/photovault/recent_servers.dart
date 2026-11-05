// PHOTOVAULT: server connection (new file).
//
// "Recent servers": every server the app connected to, with when it last
// connected and the last address that answered. Stored by
// `PhotovaultServerService` next to the saved server list (SharedPreferences
// key `photovault.recent`, a map from the normalised public URL to
// `{"connectedAt": <ms since epoch>, "address": <base URL>}`).
//
// Data saved before this existed has no entry: those servers sort after the
// timed ones (the active server first, then the stored order), and their
// last address falls back to the older `photovault.reachedAt` map.
//
// Pure Dart; unit-tested in `test/photovault/recent_servers_test.dart`.

import "dart:convert";

import "package:photos/services/photovault/server_address.dart";
import "package:photos/services/photovault/server_address_selector.dart";
import "package:photos/services/photovault/server_contract.dart";

/// The key a server is stored under: its normalised public URL.
String recentServerKey(String publicUrl) =>
    tryNormalizeServerAddress(publicUrl) ?? publicUrl;

/// One stored `photovault.recent` value.
class RecentServerEntry {
  const RecentServerEntry({this.connectedAt, this.address});

  final DateTime? connectedAt;

  /// Normalised base URL that last answered.
  final String? address;

  Map<String, Object?> toJson() => {
    if (connectedAt != null) "connectedAt": connectedAt!.millisecondsSinceEpoch,
    if (address != null) "address": address,
  };

  static RecentServerEntry? fromJson(Object? json) {
    if (json is! Map) return null;
    final at = json["connectedAt"];
    final address = json["address"];
    return RecentServerEntry(
      connectedAt: at is int
          ? DateTime.fromMillisecondsSinceEpoch(at, isUtc: true)
          : null,
      address: address is String ? tryNormalizeServerAddress(address) : null,
    );
  }
}

/// Reads the stored map; anything unreadable is dropped, never thrown.
Map<String, RecentServerEntry> parseRecentEntries(String? raw) {
  if (raw == null) return {};
  try {
    final map = jsonDecode(raw);
    if (map is! Map) return {};
    final out = <String, RecentServerEntry>{};
    for (final e in map.entries) {
      final key = e.key;
      final entry = RecentServerEntry.fromJson(e.value);
      if (key is String && entry != null) out[key] = entry;
    }
    return out;
  } catch (_) {
    return {};
  }
}

String encodeRecentEntries(Map<String, RecentServerEntry> entries) =>
    jsonEncode({for (final e in entries.entries) e.key: e.value.toJson()});

class RecentServer {
  const RecentServer({
    required this.info,
    this.lastConnectedAt,
    this.lastAddress,
  });

  final PhotovaultServerInfo info;

  /// Null for a server saved before connection times were kept.
  final DateTime? lastConnectedAt;

  /// The last address that answered, normalised, if known.
  final String? lastAddress;

  /// Addresses to try when the person taps the server: the last one that
  /// answered, then public, then home.
  List<String> get connectOrder =>
      _distinct([lastAddress, info.publicUrl, info.homeUrl]);

  /// Addresses to probe directly while a VPN is up: the last one that
  /// answered, then home, then public.
  List<String> get directProbeOrder =>
      _distinct([lastAddress, info.homeUrl, info.publicUrl]);
}

List<String> _distinct(List<String?> addresses) {
  final out = <String>[];
  for (final a in addresses) {
    if (a == null || a.trim().isEmpty) continue;
    if (out.any((o) => sameServerAddress(o, a) || o == a)) continue;
    out.add(a);
  }
  return out;
}

/// [saved] in "Recent servers" order: most recently connected first, then
/// the servers without a time (older data): [activeUrl] first, then the
/// stored order.
List<RecentServer> orderRecentServers(
  List<PhotovaultServerInfo> saved,
  Map<String, RecentServerEntry> entries, {
  Map<String, String> reachedAt = const {},
  String? activeUrl,
}) {
  final indexed = <(int, RecentServer)>[];
  for (var i = 0; i < saved.length; i++) {
    final info = saved[i];
    final key = recentServerKey(info.publicUrl);
    final entry = entries[key];
    indexed.add((
      i,
      RecentServer(
        info: info,
        lastConnectedAt: entry?.connectedAt,
        lastAddress:
            entry?.address ?? tryNormalizeServerAddress(reachedAt[key]),
      ),
    ));
  }
  bool isActive(RecentServer s) =>
      sameServerAddress(s.info.publicUrl, activeUrl);
  indexed.sort((a, b) {
    final ta = a.$2.lastConnectedAt;
    final tb = b.$2.lastConnectedAt;
    if (ta != null && tb != null) {
      final byTime = tb.compareTo(ta);
      if (byTime != 0) return byTime;
    } else if (ta != null) {
      return -1;
    } else if (tb != null) {
      return 1;
    } else {
      final aa = isActive(a.$2);
      final ab = isActive(b.$2);
      if (aa != ab) return aa ? -1 : 1;
    }
    return a.$1.compareTo(b.$1);
  });
  return [for (final e in indexed) e.$2];
}

/// A recent server that answered a direct probe.
class DirectlyReachedServer {
  const DirectlyReachedServer({
    required this.server,
    required this.info,
    required this.address,
  });

  final RecentServer server;

  /// What the server answered (fresh name and version).
  final PhotovaultServerInfo info;

  /// The normalised address that answered.
  final String address;
}

/// Probes each Photovault server in [servers] over HTTP
/// (`/photovault/info`), trying [RecentServer.directProbeOrder] one address
/// at a time with [timeout] each, and returns those that answered as the
/// same server (same public URL), in the order of [servers]. Servers are
/// probed in parallel. Stock (compatible) servers are skipped: they do not
/// describe themselves.
Future<List<DirectlyReachedServer>> probeRecentServersDirectly(
  List<RecentServer> servers,
  ServerInfoProbe probe, {
  Duration timeout = kHomeProbeTimeout,
}) async {
  Future<DirectlyReachedServer?> one(RecentServer server) async {
    if (!server.info.isPhotovault) return null;
    for (final address in server.directProbeOrder) {
      final normalized = tryNormalizeServerAddress(address);
      if (normalized == null) continue;
      try {
        final answer = await probe(normalized, timeout).timeout(timeout);
        if (answer != null &&
            answer.isPhotovault &&
            sameServerAddress(answer.publicUrl, server.info.publicUrl)) {
          return DirectlyReachedServer(
            server: server,
            info: answer,
            address: normalized,
          );
        }
      } catch (_) {
        // Next address.
      }
    }
    return null;
  }

  final results = await Future.wait(servers.map(one));
  return results.whereType<DirectlyReachedServer>().toList();
}
