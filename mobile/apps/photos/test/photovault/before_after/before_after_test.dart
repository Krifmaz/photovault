// PHOTOVAULT: before/after screenshots of the Ente screens Photovault
// changes. The same file runs on this tree and on the pristine Ente import
// (scripts/photovault/render_before_after.sh copies it into a temporary
// upstream worktree, with tree_env_upstream.dart as tree_env.dart), so it
// only uses widgets and constructors that exist in both.
//
// Phone frame: 1080x2400 pixels at device pixel ratio 2.625 (411x914 dp),
// with a 24 dp status bar. Fonts come from FontManifest.json (Inter and the
// ente_components fonts), so text is real, not test boxes.
//
// Smoke test without PHOTOVAULT_SCREENSHOT_DIR; with it, writes
// <screen>-<light|dark>.png there.

import "dart:convert";
import "dart:io";
import "dart:ui" as ui;

import "package:dio/dio.dart";
import "package:ente_base/models/accounts.dart";
import "package:ente_strings/ente_strings.dart";
import "package:flutter/material.dart";
import "package:flutter/rendering.dart";
import "package:flutter/services.dart";
import "package:flutter_secure_storage/flutter_secure_storage.dart";
import "package:flutter_test/flutter_test.dart";
import "package:package_info_plus/package_info_plus.dart";
import "package:photos/core/configuration.dart";
import "package:photos/core/errors.dart";
import "package:photos/ente_theme_data.dart";
import "package:photos/models/user_details.dart";
import "package:photos/service_locator.dart";
import "package:photos/services/account/user_service.dart";
import "package:photos/src/rust/frb_generated.dart";
import "package:photos/states/user_details_state.dart";
import "package:photos/ui/account/email_entry_page.dart";
import "package:photos/ui/account/login_page.dart";
import "package:photos/ui/account/ott_verification_page.dart";
import "package:photos/ui/account/password_entry_page.dart";
import "package:photos/ui/account/password_reentry_page.dart";
import "package:photos/ui/account/recovery_key_page.dart";
import "package:photos/ui/common/user_dialogs.dart";
import "package:photos/ui/components/empty_state_component.dart";
import "package:photos/ui/family/family_plan_page.dart";
import "package:photos/ui/home/grant_permissions_widget.dart";
import "package:photos/ui/home/header_error_widget.dart";
import "package:photos/ui/home/landing_page_widget.dart";
import "package:photos/ui/home/loading_photos_widget.dart";
import "package:photos/ui/settings/account/account_settings_page.dart";
import "package:photos/ui/settings/backup/free_space_options.dart";
import "package:photos/ui/settings/ml/machine_learning_settings_page.dart";
import "package:photos/ui/settings/security/security_settings_page.dart";
import "package:photos/ui/settings/streaming/video_streaming_settings_page.dart";
import "package:photos/ui/settings_page.dart";
import "package:photos/ui/social/widgets/feed_empty_state.dart";
import "package:photos/ui/tabs/albums/empty_states/on_device_empty_state.dart";
import "package:photos/ui/tabs/albums/empty_states/received_empty_state.dart";
import "package:photos/ui/tabs/albums/empty_states/shared_empty_state.dart";
import "package:photos/ui/wrapped/wrapped_rewind_banner_button.dart";
import "package:shared_preferences/shared_preferences.dart";

import "tree_env.dart";

const _outEnv = "PHOTOVAULT_SCREENSHOT_DIR";
const _pixelRatio = 2.625;
const _physicalSize = Size(1080, 2400);
const _statusBar = 24.0;
const _email = "alex@example.com";

final _boundary = GlobalKey();

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

class _FakeAccountsClient extends Fake implements AccountsClient {
  @override
  Future<void> sendOtp({
    required String email,
    required String purpose,
    required bool mobile,
  }) async {}
}

class _FakeRustApi extends Fake implements EntePhotosRustApi {
  @override
  AccountsClient enteFrbLibAccountsCreateAccountsClient({
    required AccountsClientConfig config,
  }) => _FakeAccountsClient();

  @override
  String enteFrbLibRecoveryRecoveryKeyToMnemonic({
    required String recoveryKeyHex,
  }) =>
      "abandon ability able about above absent absorb abstract absurd abuse "
      "access accident account accuse achieve acid acoustic acquire across "
      "act action actor actress actual";
}

/// Plugins some pages touch on open (keep-awake on the ML page, device
/// health events): answer them so the page renders as on a phone.
void _mockPlugins() {
  final messenger =
      TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger;
  messenger.setMockMessageHandler(
    "dev.flutter.pigeon.wakelock_plus_platform_interface.WakelockPlusApi.toggle",
    (_) async => const StandardMessageCodec().encodeMessage(<Object?>[null]),
  );
  messenger.setMockMessageHandler(
    "dev.flutter.pigeon.wakelock_plus_platform_interface.WakelockPlusApi.isEnabled",
    (_) async => const StandardMessageCodec().encodeMessage(<Object?>[false]),
  );
  messenger.setMockMethodCallHandler(
    const MethodChannel("io.ente.photos.platform/device_health_events"),
    (_) async => null,
  );
}

/// A signed-in account (token and key in the mocked secure storage) so
/// Settings shows what a logged-in user sees.
Future<void> _initApp() async {
  EntePhotosRust.initMock(api: _FakeRustApi());
  PackageInfo.setMockInitialValues(
    appName: "Photos",
    packageName: "photos",
    version: "1.2.0",
    buildNumber: "1",
    buildSignature: "",
  );
  SharedPreferences.setMockInitialValues({
    Configuration.emailKey: _email,
    Configuration.userIDKey: 1,
    Configuration.tokenKey: "token",
  });
  FlutterSecureStorage.setMockInitialValues({Configuration.keyKey: "a2V5"});
  final prefs = await SharedPreferences.getInstance();
  ServiceLocator.instance.init(
    prefs,
    Dio(BaseOptions(headers: {HttpHeaders.userAgentHeader: "Photos"})),
    Dio(),
    Dio(),
    PackageInfo(
      appName: "Photos",
      packageName: "photos",
      version: "1.2.0",
      buildNumber: "1",
    ),
  );
  const pathProvider = MethodChannel("plugins.flutter.io/path_provider");
  final directory = Directory.systemTemp.createTempSync("before-after");
  TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
      .setMockMethodCallHandler(pathProvider, (call) async => directory.path);
  try {
    await Configuration.instance.init(prefs);
  } on MissingPluginException catch (_) {}
  await UserService.instance.init();
  // The ML page starts the compute controller, whose device-health timer
  // would outlive a test; start it here, on the real clock, instead.
  await computeController.init();
  await prepareTree(prefs);
}

UserDetails _userDetails() => UserDetails.fromMap({
  "email": _email,
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

Widget _withUserDetails(Widget child) => InheritedUserDetails(
  userDetails: _userDetails(),
  isCached: false,
  userDetailsState: UserDetailsStateWidgetState(),
  child: child,
);

Widget _app(Brightness brightness, Widget home) {
  return MaterialApp(
    debugShowCheckedModeBanner: false,
    theme: brightness == Brightness.light ? lightThemeData : darkThemeData,
    localizationsDelegates: StringsLocalizations.localizationsDelegates,
    supportedLocales: StringsLocalizations.supportedLocales,
    builder: (context, child) => RepaintBoundary(key: _boundary, child: child),
    home: home,
  );
}

/// An empty gallery page as the app shows it: title bar and the state.
Widget _emptyPage(String title, Widget state) => Scaffold(
  appBar: AppBar(title: Text(title)),
  body: state,
);

/// Ente's Rive-duck empty states (trash, archive, links, album search).
Widget _riveEmptyPage(
  String title,
  String asset,
  Size size,
  String Function(StringsLocalizations strings) text, {
  double textWidth = 285,
}) => Builder(
  builder: (context) => _emptyPage(
    title,
    EmptyStateComponent(
      assetPath: asset,
      animated: true,
      illustrationSize: size,
      title: text(context.strings),
      textWidth: textWidth,
    ),
  ),
);

Future<void> _pumpFrames(WidgetTester tester, [int frames = 12]) async {
  for (var i = 0; i < frames; i++) {
    await tester.pump(const Duration(milliseconds: 100));
  }
}

/// Image.asset decodes outside the fake clock; wait for every image.
Future<void> _loadImages(WidgetTester tester) async {
  await tester.runAsync(() async {
    for (final element in find.byType(Image).evaluate()) {
      final image = element.widget as Image;
      await precacheImage(image.image, element);
    }
  });
  await tester.pump();
}

Future<void> _shoot(WidgetTester tester, String name) async {
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

class _Screen {
  const _Screen(this.name, this.build, {this.then});

  final String name;
  final Widget Function() build;

  /// Runs after the first frames, e.g. to open a dialog.
  final Future<void> Function(WidgetTester tester)? then;
}

final _screens = <_Screen>[
  _Screen("landing", () => LandingPageWidget(onStartWithoutAccount: () {})),
  const _Screen("sign-in", LoginPage.new),
  _Screen("create-account", () => const EmailEntryPage()),
  _Screen(
    "verify-email",
    () => const OTTVerificationPage(_email, isCreateAccountScreen: true),
  ),
  _Screen(
    "set-password",
    () => const PasswordEntryPage(mode: PasswordEntryMode.set),
  ),
  _Screen("enter-password", () => const PasswordReentryPage()),
  _Screen(
    "recovery-key",
    () => RecoveryKeyPage(
      "3a1f9c0e7b2d4f6a8c0e2b4d6f8a0c2e4b6d8f0a2c4e6b8d0f2a4c6e8b0d2f4a",
      "Continue",
      onDone: () {},
    ),
  ),
  _Screen(
    "settings",
    () => _withUserDetails(SettingsPage(emailNotifier: ValueNotifier(_email))),
  ),
  _Screen(
    "settings-bottom",
    () => _withUserDetails(SettingsPage(emailNotifier: ValueNotifier(_email))),
    then: (tester) async {
      await tester.drag(find.byType(Scrollable).first, const Offset(0, -4000));
    },
  ),
  _Screen(
    "account-settings",
    () => _withUserDetails(const AccountSettingsPage()),
  ),
  _Screen(
    "family",
    () => FamilyPlanPage(
      initialUserDetails: _userDetails(),
      refreshOnOpen: false,
    ),
  ),
  _Screen("free-up-space", () => const FreeUpSpaceOptionsScreen()),
  _Screen("security-settings", () => const SecuritySettingsPage()),
  _Screen("video-streaming-settings", () => const VideoStreamingSettingsPage()),
  _Screen("ml-settings", () => const MachineLearningSettingsPage()),
  _Screen(
    "backup-storage-full-banner",
    () => Scaffold(
      body: SafeArea(
        child: Column(
          children: [
            HeaderErrorWidget(error: StorageLimitExceededError()),
            HeaderErrorWidget(error: NoActiveSubscriptionError()),
          ],
        ),
      ),
    ),
  ),
  _Screen("grant-permissions", () => const GrantPermissionsWidget()),
  _Screen("loading-photos", () => const LoadingPhotosWidget()),
  _Screen(
    "search-no-results",
    () => _riveEmptyPage(
      "Albums",
      "assets/ente-nomatch-ducky.riv",
      const Size(181, 138),
      (s) => s.noMatchesFound,
      textWidth: 240,
    ),
  ),
  _Screen(
    "trash-empty",
    () => _riveEmptyPage(
      "Trash",
      "assets/ente-trash-ducky.riv",
      const Size(204, 161),
      (s) => s.deletedItemsStayHereForThirtyDays,
    ),
  ),
  _Screen(
    "archive-empty",
    () => _riveEmptyPage(
      "Archive",
      "assets/ente-archive-ducky.riv",
      const Size(204, 173),
      (s) => s.archivedItemsWillShowUpHere,
    ),
  ),
  _Screen(
    "links-empty",
    () => _riveEmptyPage(
      "Links",
      "assets/ente-links-ducky.riv",
      const Size(204, 171),
      (s) => s.activeLinksWillShowUpHere,
    ),
  ),
  _Screen(
    "albums-received-empty",
    () => _emptyPage("Albums", const ReceivedEmptyState()),
  ),
  _Screen(
    "albums-shared-empty",
    () => _emptyPage("Albums", const SharedEmptyState()),
  ),
  _Screen(
    "albums-on-device-empty",
    () => _emptyPage("Albums", const OnDeviceEmptyState.noFolders()),
  ),
  _Screen("feed-empty", () => _emptyPage("Feed", const FeedEmptyState())),
  _Screen(
    "rewind-banner",
    () => const Scaffold(
      body: SafeArea(
        child: Padding(
          padding: EdgeInsets.all(16),
          child: Column(children: [WrappedRewindBannerButton()]),
        ),
      ),
    ),
  ),
  _Screen(
    "invite-dialog",
    () => const Scaffold(body: SizedBox.expand()),
    then: (tester) async {
      final context = tester.element(find.byType(Scaffold).first);
      // ignore: unawaited_futures
      showInviteDialog(context, "sam@example.com");
    },
  ),
];

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();

  setUpAll(() async {
    await _loadFonts();
    _mockPlugins();
    await _initApp();
  });

  for (final brightness in Brightness.values) {
    final mode = brightness.name;
    for (final screen in _screens) {
      testWidgets("${screen.name} ($mode, $treeName)", (tester) async {
        tester.view.physicalSize = _physicalSize;
        tester.view.devicePixelRatio = _pixelRatio;
        tester.view.padding = const FakeViewPadding(
          top: _statusBar * _pixelRatio,
        );
        addTearDown(tester.view.reset);
        await tester.pumpWidget(_app(brightness, screen.build()));
        await _pumpFrames(tester);
        if (screen.then != null) {
          await screen.then!(tester);
          await _pumpFrames(tester);
        }
        await _loadImages(tester);
        await _pumpFrames(tester, 2);
        // Layout warnings or missing native plugins must not hide the
        // picture; they are printed for the log.
        Object? error;
        while ((error = tester.takeException()) != null) {
          debugPrint("[${screen.name}/$mode] $error");
        }
        await _shoot(tester, "${screen.name}-$mode");
        // Let pending timers and animations finish before the next screen.
        await tester.pumpWidget(const SizedBox.shrink());
        await tester.pump(const Duration(minutes: 2));
        while (tester.takeException() != null) {}
      });
    }
  }
}
