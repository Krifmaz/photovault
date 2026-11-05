// PHOTOVAULT: server connection (new file).
//
// Tells whether Tailscale is up on this device: some network interface has
// a Tailscale address, IPv4 100.64.0.0/10 or IPv6 fd7a:115c:a1e0::/48.
// mDNS does not cross the VPN, so while it is up the app probes the last
// known server address directly instead.
//
// 100.64.0.0/10 is also the carrier-grade NAT range some mobile networks
// hand out, so the answer can be a false "yes" on cellular. That only costs
// a few short probes (kHomeProbeTimeout), never a wrong server: every probe
// checks the server's public URL.
//
// Pure Dart; unit-tested in `test/photovault/tailscale_test.dart`.

import "dart:io";

/// True for an address Tailscale assigns: IPv4 100.64.0.0/10 or IPv6
/// fd7a:115c:a1e0::/48. Anything unparsable is false.
bool isTailscaleAddress(String address) {
  var text = address.trim();
  // Scoped IPv6 literals (fe80::1%wlan0) are not Tailscale's, but drop the
  // zone so they parse.
  final zone = text.indexOf("%");
  if (zone >= 0) text = text.substring(0, zone);
  final ip = InternetAddress.tryParse(text);
  if (ip == null) return false;
  final b = ip.rawAddress;
  if (ip.type == InternetAddressType.IPv4) {
    return b.length == 4 && b[0] == 100 && b[1] >= 64 && b[1] <= 127;
  }
  if (ip.type == InternetAddressType.IPv6) {
    return b.length == 16 &&
        b[0] == 0xfd &&
        b[1] == 0x7a &&
        b[2] == 0x11 &&
        b[3] == 0x5c &&
        b[4] == 0xa1 &&
        b[5] == 0xe0;
  }
  return false;
}

/// True when any of [addresses] is a Tailscale address.
bool hasTailscaleAddress(Iterable<String> addresses) =>
    addresses.any(isTailscaleAddress);

/// Looks at this device's network interfaces. Best effort: false on error.
Future<bool> detectTailscale() async {
  try {
    final interfaces = await NetworkInterface.list(
      includeLoopback: false,
      includeLinkLocal: false,
    );
    return hasTailscaleAddress([
      for (final i in interfaces)
        for (final a in i.addresses) a.address,
    ]);
  } catch (_) {
    return false;
  }
}
