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
      client: PhotovaultServerInfoClient(
        clientFactory: () => MockClient((req) async {
          asked.add(req.url.host);
          if (req.url.host != "10.0.1.7") {
            throw const SocketException("Network is unreachable");
          }
          return http.Response(
            jsonEncode({
              "product": "photovault",
              "name": "Family photos",
              "version": "0.4.0",
              "publicUrl": "https://photos.example.com",
              "homeUrl": "http://10.0.1.7:8095",
              "signupsAllowed": true,
            }),
            200,
          );
        }),
      ),
    );
    final payload = parseConnectPayload(
      "photovault://connect?url=https%3A%2F%2Fphotos.example.com"
      "&home=http%3A%2F%2F10.0.1.7%3A8095&name=Family",
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
    expect(asked, contains("10.0.1.7"));
    expect(find.text("Family photos"), findsOneWidget);
    expect(find.text(PhotovaultStrings.continueLabel), findsOneWidget);
    final field = tester.widget<TextField>(find.byType(TextField));
    expect(field.controller!.text, "http://10.0.1.7:8095");
  });
}
