// PHOTOVAULT: server connection (new file).
//
// Finds Photovault servers on the local network through DNS-SD
// (`_photovault._tcp`) with the pure-Dart `multicast_dns` package, the same
// way Ente's Chromecast search does (`plugins/ente_cast`), holding the same
// Android Wi-Fi multicast lock. Best effort: errors give an empty list.
//
// iOS: raw multicast needs Apple's multicast entitlement, so discovery is
// skipped there (Ente's cast search uses a native Bonjour channel instead).

import "dart:async";
import "dart:io";

import "package:ente_cast/ente_cast.dart" show withMulticastLock;
import "package:logging/logging.dart";
import "package:multicast_dns/multicast_dns.dart";
import "package:photos/services/photovault/server_contract.dart";

class PhotovaultServerDiscovery {
  const PhotovaultServerDiscovery();

  static final _logger = Logger("PhotovaultServerDiscovery");
  static const _serviceDomain = "$kPhotovaultMdnsService.local";

  bool get isSupported => Platform.isAndroid;

  Future<List<DiscoveredServer>> search({
    Duration timeout = const Duration(seconds: 4),
  }) async {
    if (!isSupported) return const [];
    try {
      return await withMulticastLock(() => _search(timeout));
    } catch (e, s) {
      _logger.warning("mDNS search failed", e, s);
      return const [];
    }
  }

  Future<List<DiscoveredServer>> _search(Duration timeout) async {
    final client = MDnsClient();
    final found = <String, DiscoveredServer>{};
    final deadline = DateTime.now().add(timeout);
    await client.start();
    try {
      final tasks = <Future<void>>[];
      await for (final ptr in client.lookup<PtrResourceRecord>(
        ResourceRecordQuery.serverPointer(_serviceDomain),
        timeout: _remaining(deadline),
      )) {
        tasks.add(_resolve(client, ptr.domainName, deadline, found));
      }
      await Future.wait(tasks);
    } finally {
      client.stop();
    }
    final list = found.values.toList()
      ..sort((a, b) => a.name.toLowerCase().compareTo(b.name.toLowerCase()));
    return list;
  }

  Future<void> _resolve(
    MDnsClient client,
    String instance,
    DateTime deadline,
    Map<String, DiscoveredServer> found,
  ) async {
    final txt = <String, String>{};
    await for (final record in client.lookup<TxtResourceRecord>(
      ResourceRecordQuery.text(instance),
      timeout: _remaining(deadline),
    )) {
      for (final entry in record.text.split("\n")) {
        final i = entry.indexOf("=");
        if (i > 0) txt[entry.substring(0, i)] = entry.substring(i + 1);
      }
    }
    await for (final srv in client.lookup<SrvResourceRecord>(
      ResourceRecordQuery.service(instance),
      timeout: _remaining(deadline),
    )) {
      String? ip;
      await for (final a in client.lookup<IPAddressResourceRecord>(
        ResourceRecordQuery.addressIPv4(srv.target),
        timeout: _remaining(deadline),
      )) {
        ip = a.address.address;
        break;
      }
      // Without an A record, the TXT addresses come first, then the SRV
      // host name.
      final server =
          discoveredServerFromMdns(
            instanceName: instance,
            txt: txt,
            ip: ip,
            port: srv.port,
          ) ??
          discoveredServerFromMdns(
            instanceName: instance,
            txt: txt,
            ip: srv.target.replaceAll(RegExp(r"\.$"), ""),
            port: srv.port,
          );
      if (server != null) found[instance] = server;
      return;
    }
    // No SRV answer: fall back to the TXT addresses.
    final server = discoveredServerFromMdns(instanceName: instance, txt: txt);
    if (server != null) found[instance] = server;
  }

  Duration _remaining(DateTime deadline) {
    final remaining = deadline.difference(DateTime.now());
    return remaining > Duration.zero ? remaining : Duration.zero;
  }
}
