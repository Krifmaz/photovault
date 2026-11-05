// PHOTOVAULT: server connection tests (pure Dart).

import "package:flutter_test/flutter_test.dart";
import "package:photos/services/photovault/recent_servers.dart";
import "package:photos/services/photovault/server_contract.dart";

const _tailnet = "http://100.101.102.103:8080";

PhotovaultServerInfo _server(String host, {String? home}) =>
    PhotovaultServerInfo(
      name: host,
      version: "0.4.0",
      publicUrl: "https://$host",
      homeUrl: home,
    );

final _a = _server("a.example.com", home: "http://192.168.1.10:8080");
final _b = _server("b.example.com");
final _c = _server("c.example.com");

DateTime _t(int day) => DateTime.utc(2026, 11, day);

void main() {
  group("order", () {
    test("most recently connected first", () {
      final list = orderRecentServers(
        [_a, _b, _c],
        {
          "https://a.example.com": RecentServerEntry(connectedAt: _t(1)),
          "https://b.example.com": RecentServerEntry(connectedAt: _t(3)),
          "https://c.example.com": RecentServerEntry(connectedAt: _t(2)),
        },
      );
      expect(list.map((s) => s.info.name), [
        "b.example.com",
        "c.example.com",
        "a.example.com",
      ]);
      expect(list.first.lastConnectedAt, _t(3));
    });

    test("servers saved before times were kept come last, active first", () {
      final list = orderRecentServers(
        [_a, _b, _c],
        {"https://c.example.com": RecentServerEntry(connectedAt: _t(2))},
        activeUrl: "https://b.example.com/",
      );
      expect(list.map((s) => s.info.name), [
        "c.example.com",
        "b.example.com",
        "a.example.com",
      ]);
    });

    test("keeps the stored order without any times", () {
      final list = orderRecentServers([_c, _a, _b], const {});
      expect(list.map((s) => s.info.name), [
        "c.example.com",
        "a.example.com",
        "b.example.com",
      ]);
      expect(list.every((s) => s.lastConnectedAt == null), isTrue);
    });

    test("entries of forgotten servers are ignored", () {
      final list = orderRecentServers(
        [_a],
        {"https://gone.example.com": RecentServerEntry(connectedAt: _t(9))},
      );
      expect(list.single.info, _a);
    });
  });

  group("stored data", () {
    test("round trip", () {
      final entries = {
        "https://a.example.com": RecentServerEntry(
          connectedAt: _t(4),
          address: _tailnet,
        ),
      };
      final back = parseRecentEntries(encodeRecentEntries(entries));
      expect(back.keys, ["https://a.example.com"]);
      expect(back.values.single.connectedAt, _t(4));
      expect(back.values.single.address, _tailnet);
    });

    test("unreadable data gives no entries instead of throwing", () {
      expect(parseRecentEntries(null), isEmpty);
      expect(parseRecentEntries("not json"), isEmpty);
      expect(parseRecentEntries("[1, 2]"), isEmpty);
      final partly = parseRecentEntries(
        '{"https://a.example.com": {"connectedAt": "yesterday"}, "x": 3}',
      );
      expect(partly.keys, ["https://a.example.com"]);
      expect(partly.values.single.connectedAt, isNull);
    });

    test("older data: last address comes from reachedAt", () {
      final list = orderRecentServers(
        [_b],
        const {},
        reachedAt: {"https://b.example.com": _tailnet},
      );
      expect(list.single.lastAddress, _tailnet);
    });

    test("a stored address wins over reachedAt", () {
      final list = orderRecentServers(
        [_a],
        {
          "https://a.example.com": RecentServerEntry(
            connectedAt: _t(1),
            address: "http://192.168.1.10:8080",
          ),
        },
        reachedAt: {"https://a.example.com": _tailnet},
      );
      expect(list.single.lastAddress, "http://192.168.1.10:8080");
    });
  });

  group("address order", () {
    test("tap: last address, then public, then home", () {
      final s = RecentServer(info: _a, lastAddress: _tailnet);
      expect(s.connectOrder, [
        _tailnet,
        "https://a.example.com",
        "http://192.168.1.10:8080",
      ]);
    });

    test("direct probe: last address, then home, then public", () {
      final s = RecentServer(info: _a, lastAddress: _tailnet);
      expect(s.directProbeOrder, [
        _tailnet,
        "http://192.168.1.10:8080",
        "https://a.example.com",
      ]);
    });

    test("no duplicates when the last address is the public one", () {
      final s = RecentServer(info: _b, lastAddress: "https://b.example.com/");
      expect(s.connectOrder, ["https://b.example.com/"]);
      expect(RecentServer(info: _b).directProbeOrder, [
        "https://b.example.com",
      ]);
    });
  });

  group("probeRecentServersDirectly", () {
    test("finds a server at its last address", () async {
      final asked = <String>[];
      final found = await probeRecentServersDirectly(
        [RecentServer(info: _b, lastAddress: _tailnet)],
        (url, timeout) async {
          asked.add(url);
          return url == _tailnet ? _b : null;
        },
      );
      expect(asked, [_tailnet]);
      expect(found.single.address, _tailnet);
      expect(found.single.info, _b);
    });

    test("falls back to home, then public", () async {
      final asked = <String>[];
      final found = await probeRecentServersDirectly(
        [RecentServer(info: _a, lastAddress: _tailnet)],
        (url, timeout) async {
          asked.add(url);
          return url == "https://a.example.com" ? _a : null;
        },
      );
      expect(asked, [
        _tailnet,
        "http://192.168.1.10:8080",
        "https://a.example.com",
      ]);
      expect(found.single.address, "https://a.example.com");
    });

    test("ignores another server answering at the address", () async {
      final found = await probeRecentServersDirectly([
        RecentServer(info: _b, lastAddress: _tailnet),
      ], (url, timeout) async => _c);
      expect(found, isEmpty);
    });

    test("skips servers that do not describe themselves", () async {
      var probes = 0;
      final found = await probeRecentServersDirectly(
        [
          RecentServer(
            info: PhotovaultServerInfo.compatible("http://192.168.1.12:8080"),
          ),
        ],
        (url, timeout) async {
          probes++;
          return null;
        },
      );
      expect(found, isEmpty);
      expect(probes, 0);
    });

    test("short timeout per address, errors are a miss", () async {
      Duration? seen;
      final found = await probeRecentServersDirectly(
        [RecentServer(info: _b, lastAddress: _tailnet), RecentServer(info: _c)],
        (url, timeout) async {
          seen = timeout;
          if (url == _tailnet) {
            await Future<void>.delayed(const Duration(milliseconds: 300));
            return _b;
          }
          if (url == "https://b.example.com") throw Exception("no route");
          return _c;
        },
        timeout: const Duration(milliseconds: 20),
      );
      expect(seen, const Duration(milliseconds: 20));
      expect(found.map((f) => f.info.name), ["c.example.com"]);
    });

    test("keeps the recent order", () async {
      final found = await probeRecentServersDirectly(
        [RecentServer(info: _c), RecentServer(info: _b)],
        (url, timeout) async {
          if (url.contains("b.")) return _b;
          await Future<void>.delayed(const Duration(milliseconds: 10));
          return _c;
        },
      );
      expect(found.map((f) => f.info.name), ["c.example.com", "b.example.com"]);
    });
  });
}
