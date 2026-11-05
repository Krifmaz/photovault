// PHOTOVAULT: server connection tests. "Recent servers" storage, migration
// of older saved data, and the Tailscale direct probe in
// PhotovaultServerService.
import "dart:convert";
import "dart:io";

import "package:flutter_test/flutter_test.dart";
import "package:http/http.dart" as http;
import "package:http/testing.dart";
import "package:photos/core/network/endpoint_config.dart";
import "package:photos/services/photovault/photovault_server_service.dart";
import "package:photos/services/photovault/server_contract.dart";
import "package:photos/services/photovault/server_info_client.dart";
import "package:shared_preferences/shared_preferences.dart";

const _tailnet = "http://100.101.102.103:8080";

const _family = PhotovaultServerInfo(
  name: "Family photos",
  version: "0.4.0",
  publicUrl: "https://photos.example.com",
  homeUrl: "http://192.168.1.10:8080",
);

const _work = PhotovaultServerInfo(
  name: "Work",
  version: "0.4.0",
  publicUrl: "https://work.example.com",
);

/// Answers `/photovault/info` as [server] only from the hosts in [reachable].
PhotovaultServerInfoClient _client(
  Map<String, PhotovaultServerInfo> reachable, [
  List<String>? asked,
]) => PhotovaultServerInfoClient(
  clientFactory: () => MockClient((req) async {
    final base =
        "${req.url.scheme}://${req.url.host}"
        "${req.url.hasPort ? ":${req.url.port}" : ""}";
    asked?.add(base);
    final server = reachable[base];
    if (server == null) throw const SocketException("Network is unreachable");
    return http.Response(jsonEncode(server.toJson()), 200);
  }),
);

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();
  final service = PhotovaultServerService.instance;
  late SharedPreferences prefs;
  var now = DateTime.utc(2026, 11, 1);

  Future<void> start(Map<String, Object> stored) async {
    SharedPreferences.setMockInitialValues(stored);
    prefs = await SharedPreferences.getInstance();
    service.init(prefs, EndpointConfig(prefs), watchNetwork: false);
    service.overrideForTesting(
      client: _client(const {}),
      tailscale: () async => false,
      now: () => now,
    );
  }

  test("every connection is remembered, most recent first", () async {
    await start({});
    now = DateTime.utc(2026, 11, 1);
    await service.activate(_family, answeredAddress: "photos.example.com");
    now = DateTime.utc(2026, 11, 2);
    await service.activate(_work, answeredAddress: "work.example.com");
    expect(service.recentServers.map((s) => s.info.name), [
      "Work",
      "Family photos",
    ]);
    expect(service.recentServers.first.lastConnectedAt, now);

    now = DateTime.utc(2026, 11, 3);
    await service.activate(_family, answeredAddress: _tailnet);
    final recent = service.recentServers;
    expect(recent.map((s) => s.info.name), ["Family photos", "Work"]);
    expect(recent.first.lastAddress, _tailnet);
    expect(recent.first.lastConnectedAt, now);
    expect(recent.last.lastAddress, "https://work.example.com");
  });

  test("removing a server forgets its connection record", () async {
    await start({});
    await service.activate(_family, answeredAddress: _tailnet);
    await service.forget(_family);
    expect(service.recentServers, isEmpty);
    await service.activate(_family);
    expect(service.recentServers.single.lastAddress, isNull);
  });

  test("data saved before Recent servers keeps working", () async {
    // Only what older versions stored: the list, the active server and the
    // address the person connected through.
    await start({
      "photovault.servers": jsonEncode([_work.toJson(), _family.toJson()]),
      "photovault.activeServer": _family.publicUrl,
      "photovault.reachedAt": jsonEncode({
        "https://photos.example.com": _tailnet,
      }),
    });
    final recent = service.recentServers;
    expect(recent.map((s) => s.info.name), ["Family photos", "Work"]);
    expect(recent.first.lastConnectedAt, isNull);
    expect(recent.first.lastAddress, _tailnet);
    expect(recent.first.connectOrder.first, _tailnet);

    // Connecting adds a time without losing the other server.
    now = DateTime.utc(2026, 11, 4);
    await service.activate(_work, answeredAddress: "work.example.com");
    expect(service.recentServers.map((s) => s.info.name), [
      "Work",
      "Family photos",
    ]);
    expect(service.savedServers, hasLength(2));
  });

  test("unreadable recent data falls back to the saved list", () async {
    await start({
      "photovault.servers": jsonEncode([_family.toJson()]),
      "photovault.recent": "{broken",
    });
    expect(service.recentServers.single.info.name, "Family photos");
  });

  test("Tailscale up: recent servers are probed directly", () async {
    await start({});
    await service.activate(_family, answeredAddress: _tailnet);
    await service.activate(_work, answeredAddress: "work.example.com");
    final asked = <String>[];
    service.overrideForTesting(
      client: _client({_tailnet: _family}, asked),
      tailscale: () async => true,
    );
    expect(await service.isTailscaleActive(), isTrue);
    final found = await service.probeRecentDirectly();
    expect(found.single.info.name, "Family photos");
    expect(found.single.address, _tailnet);
    expect(asked.first, isIn([_tailnet, "https://work.example.com"]));
    expect(asked, contains(_tailnet));
  });

  test("refresh lands on the last address while Tailscale is up", () async {
    await start({});
    service.overrideForTesting(client: _client({_tailnet: _family}));
    await service.activate(_family, answeredAddress: _tailnet);
    // Cellular, VPN down: nothing answers, the public URL stays.
    service.overrideForTesting(client: _client(const {}));
    await service.refreshAddress();
    expect(service.currentAddress, _family.publicUrl);

    // VPN up: home and public are out of reach, the tailnet address answers.
    service.overrideForTesting(
      client: _client({_tailnet: _family}),
      tailscale: () async => true,
    );
    await service.refreshAddress();
    expect(service.currentAddress, _tailnet);
  });

  test("a failing Tailscale check counts as down", () async {
    await start({});
    service.overrideForTesting(tailscale: () async => throw Exception("no"));
    expect(await service.isTailscaleActive(), isFalse);
  });
}
