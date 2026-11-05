// PHOTOVAULT: server connection tests (pure Dart).
import "package:flutter_test/flutter_test.dart";
import "package:photos/services/photovault/server_address_selector.dart";
import "package:photos/services/photovault/server_contract.dart";

const _public = "https://photos.example.com";
const _home = "http://192.168.1.10:8080";

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

  group("reachedAt (address the person connected through)", () {
    const lan = "http://192.168.1.20:8080";

    test("used when the public URL does not answer", () async {
      final asked = <String>[];
      final selector = ServerAddressSelector(
        probe: (url, timeout) async {
          asked.add(url);
          return url == lan ? _info(home: null) : null;
        },
      );
      expect(await selector.select(_info(home: null), reachedAt: lan), lan);
      expect(asked, [_public, lan]);
    });

    test("public wins when it answers", () async {
      final selector = ServerAddressSelector(
        probe: (url, timeout) async => _info(home: null),
      );
      expect(await selector.select(_info(home: null), reachedAt: lan), _public);
    });

    test("ignored when it reports another server", () async {
      final selector = ServerAddressSelector(
        probe: (url, timeout) async =>
            url == lan ? _info(public: "https://other.example.com") : null,
      );
      expect(await selector.select(_info(home: null), reachedAt: lan), _public);
    });

    test("home still comes first", () async {
      final selector = ServerAddressSelector(
        probe: (url, timeout) async => _info(),
      );
      expect(await selector.select(_info(), reachedAt: lan), _home);
    });

    test("no extra probe when it is the public URL", () async {
      var probes = 0;
      final selector = ServerAddressSelector(
        probe: (url, timeout) async {
          probes++;
          return null;
        },
      );
      expect(
        await selector.select(_info(home: null), reachedAt: "$_public/"),
        _public,
      );
      expect(probes, 0);
    });
  });

  group("Tailscale up: last address tried directly", () {
    const tailnet = "http://100.101.102.103:8080";

    test("after home, before public", () async {
      final asked = <String>[];
      final selector = ServerAddressSelector(
        probe: (url, timeout) async {
          asked.add(url);
          return url == tailnet ? _info() : null;
        },
      );
      expect(
        await selector.select(
          _info(),
          lastAddress: tailnet,
          reachedAt: tailnet,
          vpnActive: true,
        ),
        tailnet,
      );
      expect(asked, [_home, tailnet]);
    });

    test("home still wins", () async {
      final selector = ServerAddressSelector(
        probe: (url, timeout) async => _info(),
      );
      expect(
        await selector.select(_info(), lastAddress: tailnet, vpnActive: true),
        _home,
      );
    });

    test("public when the last address does not answer", () async {
      final asked = <String>[];
      final selector = ServerAddressSelector(
        probe: (url, timeout) async {
          asked.add(url);
          return url == _public ? _info() : null;
        },
      );
      expect(
        await selector.select(_info(), lastAddress: tailnet, vpnActive: true),
        _public,
      );
      expect(asked, [_home, tailnet, _public]);
    });

    test("not tried while the VPN is down", () async {
      final asked = <String>[];
      final selector = ServerAddressSelector(
        probe: (url, timeout) async {
          asked.add(url);
          return url == tailnet ? _info() : null;
        },
      );
      expect(await selector.select(_info(), lastAddress: tailnet), _public);
      expect(asked, [_home]);
    });

    test("no extra probe when the last address is public or home", () async {
      final asked = <String>[];
      final selector = ServerAddressSelector(
        probe: (url, timeout) async {
          asked.add(url);
          return null;
        },
      );
      expect(
        await selector.select(_info(), lastAddress: _public, vpnActive: true),
        _public,
      );
      expect(
        await selector.select(_info(), lastAddress: _home, vpnActive: true),
        _public,
      );
      expect(asked, [_home, _home]);
    });
  });
}
