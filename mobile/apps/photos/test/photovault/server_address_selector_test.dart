// PHOTOVAULT: server connection tests (pure Dart).
import "dart:async";

import "package:flutter_test/flutter_test.dart";
import "package:photos/services/photovault/server_address_selector.dart";
import "package:photos/services/photovault/server_contract.dart";

const _public = "https://photos.example.com";
const _home = "http://10.0.1.7:8095";

PhotovaultServerInfo _info({String? home = _home, String public = _public}) {
  return PhotovaultServerInfo(
    name: "Family",
    version: "0.2.0",
    publicUrl: public,
    homeUrl: home,
  );
}

void main() {
  test("uses home when it answers with the same public URL", () async {
    final asked = <String>[];
    final selector = ServerAddressSelector(
      probe: (url, timeout) async {
        asked.add(url);
        return _info(public: "$_public/");
      },
    );
    expect(await selector.select(_info()), _home);
    expect(asked, [_home]);
  });

  test("uses public when home does not answer", () async {
    final selector = ServerAddressSelector(probe: (url, timeout) async => null);
    expect(await selector.select(_info()), _public);
  });

  test("uses public when home throws", () async {
    final selector = ServerAddressSelector(
      probe: (url, timeout) async => throw Exception("no route"),
    );
    expect(await selector.select(_info()), _public);
  });

  test("uses public when home is slower than the timeout", () async {
    final selector = ServerAddressSelector(
      timeout: const Duration(milliseconds: 20),
      probe: (url, timeout) async {
        await Future<void>.delayed(const Duration(milliseconds: 300));
        return _info();
      },
    );
    expect(await selector.select(_info()), _public);
  });

  test("uses public when home is a different server", () async {
    final selector = ServerAddressSelector(
      probe: (url, timeout) async => _info(public: "https://other.example.com"),
    );
    expect(await selector.select(_info()), _public);
  });

  test("no probe without a distinct home address", () async {
    var probes = 0;
    final selector = ServerAddressSelector(
      probe: (url, timeout) async {
        probes++;
        return _info();
      },
    );
    expect(await selector.select(_info(home: null)), _public);
    expect(await selector.select(_info(home: "")), _public);
    expect(await selector.select(_info(home: "$_public/")), _public);
    expect(
      await selector.select(PhotovaultServerInfo.compatible(_home)),
      _home,
    );
    expect(probes, 0);
  });

  test("passes the contract timeout to the probe", () async {
    Duration? seen;
    final selector = ServerAddressSelector(
      probe: (url, timeout) async {
        seen = timeout;
        return null;
      },
    );
    await selector.select(_info());
    expect(seen, kHomeProbeTimeout);
  });
}
