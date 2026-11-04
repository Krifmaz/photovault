// PHOTOVAULT: email verification skip, sign-up / login flow through
// `UserService.sendOtt` with a fake accounts client.
import "package:ente_base/models/accounts.dart";
import "package:ente_strings/ente_strings.dart";
import "package:flutter/material.dart";
import "package:flutter_test/flutter_test.dart";
import "package:photos/ente_theme_data.dart";
import "package:photos/services/account/user_service.dart";
import "package:photos/services/photovault/photovault_server_service.dart";
import "package:photos/services/photovault/server_contract.dart";
import "package:photos/ui/account/ott_verification_page.dart";
import "package:photos/ui/account/two_factor_authentication_page.dart";

import "../ui/account/accounts_test_support.dart";

class _Client extends FakeAccountsClient {
  AuthResponse? response;
  AccountsError? verifyFailure;

  @override
  Future<AuthResponse> verifyEmail({
    required String email,
    required String code,
    String? source,
  }) async {
    requests.add({"email": email, "code": code});
    if (verifyFailure != null) throw verifyFailure!;
    return response ?? (throw UnimplementedError());
  }
}

PhotovaultServerInfo _server(String? emailVerification) {
  return PhotovaultServerInfo.fromJson({
    "product": "photovault",
    "name": "Home",
    "version": "0.2.0",
    "publicUrl": "https://photos.example.com",
    "emailVerification": ?emailVerification,
  })!;
}

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();
  final client = _Client();
  final active = PhotovaultServerService.instance.activeServer;

  setUpAll(() => initializeAccountsTest(client));

  setUp(() {
    client.requests.clear();
    client.failure = null;
    client.response = null;
    client.verifyFailure = null;
  });
  tearDown(() => active.value = null);

  Future<void> settle(WidgetTester tester) async {
    for (var i = 0; i < 12; i++) {
      await tester.pump(const Duration(milliseconds: 100));
    }
  }

  Future<void> start(WidgetTester tester, String purpose) async {
    await tester.pumpWidget(
      MaterialApp(
        theme: lightThemeData,
        localizationsDelegates: StringsLocalizations.localizationsDelegates,
        supportedLocales: StringsLocalizations.supportedLocales,
        home: Builder(
          builder: (context) => Scaffold(
            body: TextButton(
              onPressed: () => UserService.instance.sendOtt(
                context,
                "person@example.com",
                isCreateAccountScreen: purpose == "signup",
                purpose: purpose,
              ),
              child: const Text("Enter email"),
            ),
          ),
        ),
      ),
    );
    await tester.tap(find.text("Enter email"));
    await settle(tester);
  }

  for (final purpose in ["signup", "login"]) {
    testWidgets("$purpose: skipping server continues without the code screen", (
      tester,
    ) async {
      active.value = _server("skip");
      client.response = const AuthResponse(id: 1, twoFactorSessionId: "tfa");
      await start(tester, purpose);
      expect(client.requests, [
        {"email": "person@example.com", "purpose": purpose, "mobile": false},
        {"email": "person@example.com", "code": "000000"},
      ]);
      expect(
        find.byType(OTTVerificationPage, skipOffstage: false),
        findsNothing,
      );
      expect(find.byType(TwoFactorAuthenticationPage), findsOneWidget);
    });

    testWidgets("$purpose: rejected automatic code shows the code screen", (
      tester,
    ) async {
      active.value = _server("skip");
      client.verifyFailure = const AccountsError.incorrectEmailCode();
      await start(tester, purpose);
      expect(client.requests.last, {
        "email": "person@example.com",
        "code": "000000",
      });
      expect(find.byType(OTTVerificationPage), findsOneWidget);
      // No "Incorrect code" sheet: the person just sees the usual screen.
      expect(find.text("Incorrect code"), findsNothing);
      expect(tester.takeException(), isNull);
    });
  }

  for (final mode in [null, "smtp", "log"]) {
    testWidgets("emailVerification $mode keeps the code screen", (
      tester,
    ) async {
      active.value = _server(mode);
      await start(tester, "signup");
      expect(client.requests, hasLength(1));
      expect(find.byType(OTTVerificationPage), findsOneWidget);
    });
  }
}
