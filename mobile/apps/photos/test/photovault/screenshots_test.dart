// PHOTOVAULT: screenshots of the screens Photovault adds or changes, rendered
// with the app's real light and dark themes and the bundled fonts (Inter and
// the ente_components fonts, loaded from FontManifest.json so text is not
// drawn as test boxes).
//
// Always runs as a smoke test. To write PNGs, set PHOTOVAULT_SCREENSHOT_DIR:
//   PHOTOVAULT_SCREENSHOT_DIR=/tmp/shots flutter test test/photovault/screenshots_test.dart
// Files are named <screen>-<light|dark>.png. No golden comparison: font
// rasterisation differs between machines, so committed goldens would be flaky.

import "dart:convert";
import "dart:io";
import "dart:ui" as ui;

import "package:ente_components/ente_components.dart";
import "package:ente_strings/ente_strings.dart";
import "package:flutter/material.dart";
import "package:flutter/rendering.dart";
import "package:flutter/services.dart";
import "package:flutter_test/flutter_test.dart";
import "package:http/http.dart" as http;
import "package:http/testing.dart";
import "package:hugeicons/hugeicons.dart";
import "package:photos/core/network/endpoint_config.dart";
import "package:photos/ente_theme_data.dart";
import "package:photos/models/user_details.dart";
import "package:photos/services/photovault/photovault_server_service.dart";
import "package:photos/services/photovault/photovault_strings.dart";
import "package:photos/services/photovault/server_contract.dart";
import "package:photos/services/photovault/server_discovery.dart";
import "package:photos/services/photovault/server_info_client.dart";
import "package:photos/states/user_details_state.dart";
import "package:photos/ui/photovault/connect_server_page.dart";
import "package:photos/ui/photovault/server_qr_scan_page.dart";
import "package:photos/ui/photovault/server_settings_page.dart";
import "package:photos/ui/settings/storage_card_widget.dart";
import "package:shared_preferences/shared_preferences.dart";

const _outEnv = "PHOTOVAULT_SCREENSHOT_DIR";
// 1080x2400 pixels at 2.625 (411x914 dp), as before_after/ uses.
const _physical = Size(1080, 2400);
const _pixelRatio = 2.625;

const _family = PhotovaultServerInfo(
  name: "Family photos",
  version: "0.4.0",
  publicUrl: "https://photos.example.com",
  homeUrl: "http://192.168.1.10:8080",
);

Future<void> _loadFonts() async {
  final manifest =
      jsonDecode(await rootBundle.loadString("FontManifest.json")) as List;
  for (final entry in manifest.cast<Map<String, dynamic>>()) {
    final loader = FontLoader(entry["family"] as String);
    for (final font in (entry["fonts"] as List).cast<Map<String, dynamic>>()) {
      loader.addFont(rootBundle.load(font["asset"] as String));
    }
    await loader.load();
  }
}

class _FakeDiscovery extends PhotovaultServerDiscovery {
  const _FakeDiscovery(this.servers);

  final List<DiscoveredServer> servers;

  @override
  bool get isSupported => true;

  @override
  Future<List<DiscoveredServer>> search({
    Duration timeout = const Duration(seconds: 4),
  }) async => servers;
}

PhotovaultServerInfoClient _http(
  Future<http.Response> Function(http.Request) handler,
) => PhotovaultServerInfoClient(clientFactory: () => MockClient(handler));

final _refusing = _http(
  (_) async => throw const SocketException("Connection refused"),
);

final _answering = _http(
  (req) async => http.Response(
    jsonEncode({
      "product": "photovault",
      "name": _family.name,
      "version": _family.version,
      "publicUrl": _family.publicUrl,
      "homeUrl": _family.homeUrl,
      "signupsAllowed": true,
    }),
    200,
  ),
);

Widget _app(Brightness brightness, Widget home) {
  return MaterialApp(
    debugShowCheckedModeBanner: false,
    theme: brightness == Brightness.light ? lightThemeData : darkThemeData,
    localizationsDelegates: StringsLocalizations.localizationsDelegates,
    supportedLocales: StringsLocalizations.supportedLocales,
    home: RepaintBoundary(key: _boundary, child: home),
  );
}

final _boundary = GlobalKey();

Future<void> _pumpFrames(WidgetTester tester, [int frames = 10]) async {
  for (var i = 0; i < frames; i++) {
    await tester.pump(const Duration(milliseconds: 100));
  }
}

Future<void> _shoot(WidgetTester tester, String name) async {
  expect(tester.takeException(), isNull);
  final dir = Platform.environment[_outEnv];
  if (dir == null || dir.isEmpty) return;
  final boundary =
      _boundary.currentContext!.findRenderObject()! as RenderRepaintBoundary;
  await tester.runAsync(() async {
    final image = await boundary.toImage(pixelRatio: _pixelRatio);
    final bytes = await image.toByteData(format: ui.ImageByteFormat.png);
    image.dispose();
    await Directory(dir).create(recursive: true);
    await File("$dir/$name.png").writeAsBytes(bytes!.buffer.asUint8List());
  });
}

/// The top of Settings as a signed-in user sees it: the storage card with
/// real used/total numbers and the new "Server" entry between Backup and
/// Security (same widgets and order as `settings_page.dart`).
class _SettingsExcerpt extends StatelessWidget {
  const _SettingsExcerpt(this.userDetails);

  final UserDetails userDetails;

  @override
  Widget build(BuildContext context) {
    return InheritedUserDetails(
      userDetails: userDetails,
      isCached: false,
      userDetailsState: UserDetailsStateWidgetState(),
      child: SettingsPageScaffold(
        title: userDetails.email,
        children: [
          const StorageCardWidget(),
          const SizedBox(height: 16),
          SettingsItem(
            title: context.strings.account,
            icon: HugeIcons.strokeRoundedUser,
            onTap: () async {},
          ),
          const SizedBox(height: 8),
          SettingsItem(
            title: context.strings.backup,
            icon: HugeIcons.strokeRoundedCloudUpload,
            onTap: () async {},
          ),
          const SizedBox(height: 8),
          SettingsItem(
            title: PhotovaultStrings.server,
            subtitle: _family.name,
            icon: HugeIcons.strokeRoundedServerStack01,
            onTap: () async {},
          ),
          const SizedBox(height: 8),
          SettingsItem(
            title: context.strings.security,
            icon: HugeIcons.strokeRoundedSecurityCheck,
            onTap: () async {},
          ),
        ],
      ),
    );
  }
}

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();
  final service = PhotovaultServerService.instance;
  late SharedPreferences prefs;

  setUpAll(_loadFonts);

  setUp(() async {
    SharedPreferences.setMockInitialValues({});
    prefs = await SharedPreferences.getInstance();
    service.init(prefs, EndpointConfig(prefs), watchNetwork: false);
    service.overrideForTesting(
      client: _refusing,
      discovery: const _FakeDiscovery([]),
      tailscale: () async => false,
    );
  });

  for (final brightness in Brightness.values) {
    final mode = brightness.name;

    Future<void> setUpView(WidgetTester tester) async {
      tester.view.physicalSize = _physical;
      tester.view.devicePixelRatio = _pixelRatio;
      tester.view.padding = const FakeViewPadding(top: 24 * _pixelRatio);
      addTearDown(tester.view.reset);
    }

    testWidgets("connect screen, empty ($mode)", (tester) async {
      await setUpView(tester);
      await tester.pumpWidget(
        _app(brightness, ConnectServerPage(onUseWithoutServer: () {})),
      );
      await _pumpFrames(tester);
      expect(find.text(PhotovaultStrings.connectTitle), findsOneWidget);
      await _shoot(tester, "connect-empty-$mode");
    });

    testWidgets("connect screen, saved and found servers ($mode)", (
      tester,
    ) async {
      await setUpView(tester);
      service.overrideForTesting(
        client: _answering,
        discovery: const _FakeDiscovery([
          DiscoveredServer(
            name: "Living room NAS",
            address: "http://192.168.1.20:8080",
            version: "0.4.0",
          ),
          DiscoveredServer(
            name: "Grandma's photos",
            address: "http://192.168.1.31:8080",
            version: "0.3.2",
          ),
        ]),
      );
      await service.activate(_family);
      await service.clearActive();
      await tester.pumpWidget(
        _app(brightness, ConnectServerPage(onUseWithoutServer: () {})),
      );
      await _pumpFrames(tester);
      expect(find.text("Living room NAS"), findsOneWidget);
      expect(find.text(PhotovaultStrings.recentServers), findsOneWidget);
      expect(find.text(_family.name), findsOneWidget);
      await _shoot(tester, "connect-servers-$mode");
    });

    // On cellular with Tailscale up: mDNS finds nothing, the recent
    // server answers at its last (tailnet) address.
    testWidgets("connect screen, over Tailscale ($mode)", (tester) async {
      await setUpView(tester);
      service.overrideForTesting(client: _answering);
      await service.activate(_family, answeredAddress: "100.101.102.103:8080");
      await service.clearActive();
      service.overrideForTesting(tailscale: () async => true);
      await tester.pumpWidget(
        _app(brightness, ConnectServerPage(onUseWithoutServer: () {})),
      );
      await _pumpFrames(tester);
      expect(find.text(PhotovaultStrings.recentServers), findsOneWidget);
      expect(find.text(PhotovaultStrings.foundOverTailscale), findsOneWidget);
      expect(find.text("100.101.102.103:8080 · 0.4.0"), findsOneWidget);
      await _shoot(tester, "connect-tailscale-$mode");
    });

    testWidgets("connect screen, server answered ($mode)", (tester) async {
      await setUpView(tester);
      service.overrideForTesting(client: _answering);
      await tester.pumpWidget(_app(brightness, const ConnectServerPage()));
      await _pumpFrames(tester);
      await tester.enterText(find.byType(TextField), "192.168.1.10:8080");
      await tester.tap(find.text(PhotovaultStrings.connect));
      await _pumpFrames(tester);
      expect(find.textContaining(_family.name), findsWidgets);
      await _shoot(tester, "connect-found-$mode");
    });

    testWidgets("connect screen, unreachable ($mode)", (tester) async {
      await setUpView(tester);
      await tester.pumpWidget(_app(brightness, const ConnectServerPage()));
      await _pumpFrames(tester);
      await tester.enterText(find.byType(TextField), "192.168.1.10:8080");
      await tester.tap(find.text(PhotovaultStrings.connect));
      await _pumpFrames(tester);
      expect(find.textContaining("Can't reach"), findsOneWidget);
      await _shoot(tester, "connect-error-$mode");
    });

    testWidgets("settings: storage card and Server entry ($mode)", (
      tester,
    ) async {
      await setUpView(tester);
      final details = UserDetails.fromMap({
        "email": "alex@example.com",
        "usage": 412 * 1024 * 1024 * 1024,
        "fileCount": 48213,
        "storageBonus": 0,
        "subscription": {
          "productID": "free",
          "originalTransactionID": "",
          "paymentProvider": "",
          "storage": 2 * 1024 * 1024 * 1024 * 1024,
          "expiryTime": 0,
          "price": "",
          "period": "",
        },
      });
      StorageCardWidget.resetForTesting();
      await tester.pumpWidget(_app(brightness, _SettingsExcerpt(details)));
      await _pumpFrames(tester);
      expect(find.text(PhotovaultStrings.server), findsOneWidget);
      await _shoot(tester, "settings-$mode");
    });

    // The real disk from `/photovault/usage` (contracts/storage-usage.md):
    // whole-disk usage, the account's photos as a second line, and the
    // warning from 90% full.
    for (final (name, used, warn) in [
      ("settings-disk", 300, false),
      ("settings-disk-nearly-full", 465, true),
    ]) {
      testWidgets("settings: storage card shows the disk, $name ($mode)", (
        tester,
      ) async {
        await setUpView(tester);
        const gib = 1024 * 1024 * 1024;
        final details = UserDetails.fromMap({
          "email": "alex@example.com",
          "usage": 412 * gib ~/ 10,
          "fileCount": 48213,
          "storageBonus": 0,
          "subscription": {
            "productID": "free",
            "originalTransactionID": "",
            "paymentProvider": "",
            "storage": 500 * gib,
            "expiryTime": 0,
            "price": "",
            "period": "",
          },
        });
        StorageCardWidget.resetForTesting();
        StorageCardWidget.usageLoaderForTesting = () async => {
          "usedBytes": 412 * gib ~/ 10,
          "userBytes": 412 * gib ~/ 10,
          "limitBytes": 500 * gib,
          "freeBytes": (500 - used) * gib,
          "unlimited": false,
          "limitSource": "disk",
          "quotaBytes": null,
          "disk": {
            "totalBytes": 500 * gib,
            "usedBytes": used * gib,
            "availableBytes": (500 - used) * gib,
            "usedPercent": used / 5,
            "nearlyFull": warn,
            "source": "disk",
          },
          "nearlyFullPercent": 90,
        };
        addTearDown(StorageCardWidget.resetForTesting);
        await tester.pumpWidget(_app(brightness, _SettingsExcerpt(details)));
        await _pumpFrames(tester);
        expect(find.text("$used GB of 500 GB used"), findsOneWidget);
        expect(find.text("Your photos: 41.2 GB"), findsOneWidget);
        expect(
          find.text("Server disk is ${(used / 5).floor()}% full"),
          warn ? findsOneWidget : findsNothing,
        );
        await _shoot(tester, "$name-$mode");
      });
    }

    testWidgets("settings > Server ($mode)", (tester) async {
      await setUpView(tester);
      service.overrideForTesting(client: _refusing);
      await service.activate(_family);
      await tester.pumpWidget(_app(brightness, const ServerSettingsPage()));
      await _pumpFrames(tester);
      expect(find.text(PhotovaultStrings.switchServer), findsOneWidget);
      await _shoot(tester, "settings-server-$mode");
    });

    // Off-device the camera view is a black box (no platform view and no
    // scan frame); the page chrome around it is what this shows.
    testWidgets("QR scan page chrome ($mode)", (tester) async {
      await setUpView(tester);
      await tester.pumpWidget(_app(brightness, const ServerQrScanPage()));
      await _pumpFrames(tester);
      expect(find.text(PhotovaultStrings.scanQr), findsOneWidget);
      await _shoot(tester, "qr-scan-$mode");
    });
  }
}
