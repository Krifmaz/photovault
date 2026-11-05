// PHOTOVAULT: server connection tests (pure Dart).
import "package:flutter_test/flutter_test.dart";
import "package:photos/services/photovault/tailscale.dart";

void main() {
  test("IPv4 100.64.0.0/10 is Tailscale", () {
    for (final ip in [
      "100.64.0.1",
      "100.101.102.103",
      "100.127.255.254",
      " 100.100.100.100 ",
    ]) {
      expect(isTailscaleAddress(ip), isTrue, reason: ip);
    }
  });

  test("other IPv4 addresses are not", () {
    for (final ip in [
      "100.63.255.255",
      "100.128.0.1",
      "192.168.1.10",
      "10.64.0.1",
      "127.0.0.1",
      "8.8.8.8",
    ]) {
      expect(isTailscaleAddress(ip), isFalse, reason: ip);
    }
  });

  test("IPv6 fd7a:115c:a1e0::/48 is Tailscale", () {
    expect(isTailscaleAddress("fd7a:115c:a1e0::1"), isTrue);
    expect(
      isTailscaleAddress("FD7A:115C:A1E0:ab12:4843:cd96:6265:6667"),
      isTrue,
    );
    expect(isTailscaleAddress("fd7a:115c:a1e1::1"), isFalse);
    expect(isTailscaleAddress("fd00::7"), isFalse);
    expect(isTailscaleAddress("fe80::1%wlan0"), isFalse);
    expect(isTailscaleAddress("::1"), isFalse);
  });

  test("garbage is not", () {
    for (final text in ["", "tailscale0", "100.64", "photos.example.com"]) {
      expect(isTailscaleAddress(text), isFalse, reason: text);
    }
  });

  test("any interface address counts", () {
    expect(hasTailscaleAddress(const []), isFalse);
    expect(hasTailscaleAddress(const ["192.168.1.10", "fe80::1"]), isFalse);
    expect(
      hasTailscaleAddress(const ["192.168.1.10", "100.101.102.103"]),
      isTrue,
    );
    expect(hasTailscaleAddress(const ["fd7a:115c:a1e0::5"]), isTrue);
  });
}
