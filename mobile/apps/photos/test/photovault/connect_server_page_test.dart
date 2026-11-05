// PHOTOVAULT: server connection. Smoke test of the connect screen.
import "dart:convert";
import "dart:io";

import "package:ente_components/ente_components.dart";
import "package:flutter/material.dart";
import "package:flutter_test/flutter_test.dart";
import "package:http/http.dart" as http;
import "package:http/testing.dart";
import "package:photos/core/network/endpoint_config.dart";
import "package:photos/services/photovault/photovault_server_service.dart";
import "package:photos/services/photovault/photovault_strings.dart";
import "package:photos/services/photovault/server_contract.dart";
import "package:photos/services/photovault/server_discovery.dart";
import "package:photos/services/photovault/server_info_client.dart";
import "package:photos/ui/photovault/connect_server_page.dart";
import "package:shared_preferences/shared_preferences.dart";

class _NoDiscovery extends PhotovaultServerDiscovery {
  const _NoDiscovery();

  @override
  bool get isSupported => false;
}

void main() {
  testWidgets("renders and explains an invalid address", (tester) async {
    await tester.pumpWidget(
      MaterialApp(
        theme: ComponentTheme.lightTheme(app: ComponentApp.photos),
        home: const ConnectServerPage(),
      ),
    );
    expect(find.text(PhotovaultStrings.connectTitle), findsOneWidget);
    expect(find.text(PhotovaultStrings.connect), findsOneWidget);

    await tester.enterText(find.byType(TextField), "ftp://x");
    await tester.tap(find.text(PhotovaultStrings.connect));
    await tester.pumpAndSettle();
    expect(find.textContaining("isn't a server address"), findsOneWidget);
  });

  testWidgets("a connect link falls back to its home address", (tester) async {
    SharedPreferences.setMockInitialValues({});
    final prefs = await SharedPreferences.getInstance();
    final service = PhotovaultServerService.instance;
    service.init(prefs, EndpointConfig(prefs), watchNetwork: false);
    final asked = <String>[];
    service.overrideForTesting(
      discovery: const _NoDiscovery(),
      tailscale: () async => false,
      client: PhotovaultServerInfoClient(
        clientFactory: () => MockClient((req) async {
          asked.add(req.url.host);
          if (req.url.host != "192.168.1.10") {
            throw const SocketException("Network is unreachable");
          }
          return http.Response(
            jsonEncode({
              "product": "photovault",
              "name": "Family photos",
              "version": "0.4.0",
              "publicUrl": "https://photos.example.com",
              "homeUrl": "http://192.168.1.10:8080",
              "signupsAllowed": true,
            }),
            200,
          );
        }),
      ),
    );
    final payload = parseConnectPayload(
      "photovault://connect?url=https%3A%2F%2Fphotos.example.com"
      "&home=http%3A%2F%2F192.168.1.10%3A8080&name=Family",
    )!;
    await tester.runAsync(() async {
      await tester.pumpWidget(
        MaterialApp(
          theme: ComponentTheme.lightTheme(app: ComponentApp.photos),
          home: ConnectServerPage(initialPayload: payload),
        ),
      );
      for (var i = 0; i < 20 && asked.length < 2; i++) {
        await Future<void>.delayed(const Duration(milliseconds: 50));
        await tester.pump();
      }
      await Future<void>.delayed(const Duration(milliseconds: 50));
    });
    await tester.pump();
    expect(asked.first, "photos.example.com");
    expect(asked, contains("192.168.1.10"));
    expect(find.text("Family photos"), findsOneWidget);
    expect(find.text(PhotovaultStrings.continueLabel), findsOneWidget);
    final field = tester.widget<TextField>(find.byType(TextField));
    expect(field.controller!.text, "http://192.168.1.10:8080");
  });

  group("recent servers", () {
    const tailnet = "http://100.101.102.103:8080";
    const family = PhotovaultServerInfo(
      name: "Family photos",
      version: "0.4.0",
      publicUrl: "https://photos.example.com",
      homeUrl: "http://192.168.1.10:8080",
    );
    const work = PhotovaultServerInfo(
      name: "Work",
      version: "0.4.0",
      publicUrl: "https://work.example.com",
    );

    PhotovaultServerInfoClient answeringOnly(
      Map<String, PhotovaultServerInfo> reachable,
    ) => PhotovaultServerInfoClient(
      clientFactory: () => MockClient((req) async {
        final server = reachable["${req.url.host}:${req.url.port}"];
        if (server == null) {
          throw const SocketException("Network is unreachable");
        }
        return http.Response(jsonEncode(server.toJson()), 200);
      }),
    );

    Future<PhotovaultServerService> setUpService({
      required bool tailscale,
      Map<String, PhotovaultServerInfo> reachable = const {},
    }) async {
      SharedPreferences.setMockInitialValues({});
      final prefs = await SharedPreferences.getInstance();
      final service = PhotovaultServerService.instance;
      service.init(prefs, EndpointConfig(prefs), watchNetwork: false);
      var day = 1;
      service.overrideForTesting(
        discovery: const _NoDiscovery(),
        tailscale: () async => false,
        client: answeringOnly(const {}),
        now: () => DateTime.utc(2026, 11, day++),
      );
      await service.activate(family, answeredAddress: tailnet);
      await service.activate(work, answeredAddress: "work.example.com");
      await service.clearActive();
      service.overrideForTesting(
        tailscale: () async => tailscale,
        client: answeringOnly(reachable),
      );
      return service;
    }

    void tallView(WidgetTester tester) {
      tester.view.physicalSize = const Size(1080, 2400);
      tester.view.devicePixelRatio = 2.625;
      addTearDown(tester.view.reset);
    }

    Future<void> pumpPage(WidgetTester tester) async {
      tallView(tester);
      await tester.runAsync(() async {
        await tester.pumpWidget(
          MaterialApp(
            theme: ComponentTheme.lightTheme(app: ComponentApp.photos),
            home: const ConnectServerPage(),
          ),
        );
        for (var i = 0; i < 10; i++) {
          await Future<void>.delayed(const Duration(milliseconds: 20));
          await tester.pump();
        }
      });
      await tester.pump();
    }

    testWidgets("listed most recent first, without Tailscale section", (
      tester,
    ) async {
      await tester.runAsync(() => setUpService(tailscale: false));
      await pumpPage(tester);
      expect(find.text(PhotovaultStrings.recentServers), findsOneWidget);
      expect(find.text(PhotovaultStrings.foundOverTailscale), findsNothing);
      expect(
        tester.getTopLeft(find.text("Work")).dy,
        lessThan(tester.getTopLeft(find.text("Family photos")).dy),
      );
    });

    testWidgets("Tailscale up: the last address is probed directly", (
      tester,
    ) async {
      await tester.runAsync(
        () => setUpService(
          tailscale: true,
          reachable: {"100.101.102.103:8080": family},
        ),
      );
      await pumpPage(tester);
      expect(find.text(PhotovaultStrings.foundOverTailscale), findsOneWidget);
      expect(find.text("100.101.102.103:8080 · 0.4.0"), findsOneWidget);

      // Tapping it shows the server, ready to continue.
      await tester.tap(find.text("100.101.102.103:8080 · 0.4.0"));
      await tester.pump();
      expect(find.text(PhotovaultStrings.continueLabel), findsOneWidget);
      final field = tester.widget<TextField>(find.byType(TextField));
      expect(field.controller!.text, "100.101.102.103:8080");
    });

    testWidgets("Tailscale up, nothing answers", (tester) async {
      await tester.runAsync(() => setUpService(tailscale: true));
      await pumpPage(tester);
      expect(find.text(PhotovaultStrings.foundOverTailscale), findsOneWidget);
      expect(
        find.text(PhotovaultStrings.noneAnsweredOverTailscale),
        findsOneWidget,
      );
    });

    testWidgets("tapping a recent server tries its last address first", (
      tester,
    ) async {
      final service = await tester.runAsync(
        () => setUpService(
          tailscale: false,
          reachable: {"100.101.102.103:8080": family},
        ),
      );
      tallView(tester);
      var connected = false;
      await tester.runAsync(() async {
        await tester.pumpWidget(
          MaterialApp(
            theme: ComponentTheme.lightTheme(app: ComponentApp.photos),
            home: ConnectServerPage(onConnected: () => connected = true),
          ),
        );
        await tester.tap(find.text("Family photos"));
        for (var i = 0; i < 20 && !connected; i++) {
          await Future<void>.delayed(const Duration(milliseconds: 20));
          await tester.pump();
        }
      });
      expect(connected, isTrue);
      expect(service!.activeServer.value?.name, "Family photos");
      expect(service.recentServers.first.info.name, "Family photos");
      expect(service.recentServers.first.lastAddress, tailnet);
    });
  });
}
