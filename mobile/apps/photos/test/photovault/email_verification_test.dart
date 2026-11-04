// PHOTOVAULT: email verification skip tests (pure Dart).
import "package:flutter_test/flutter_test.dart";
import "package:photos/services/photovault/email_verification.dart";
import "package:photos/services/photovault/server_contract.dart";

PhotovaultServerInfo _server(Object? emailVerification) {
  return PhotovaultServerInfo.fromJson({
    "product": "photovault",
    "name": "Home",
    "version": "0.2.0",
    "publicUrl": "https://photos.example.com",
    "emailVerification": ?emailVerification,
  })!;
}

void main() {
  group("emailVerification field", () {
    test("parsed, case-insensitive", () {
      expect(_server("skip").emailVerification, kEmailVerificationSkip);
      expect(_server("SKIP").skipsEmailVerification, isTrue);
      expect(_server("smtp").emailVerification, kEmailVerificationSmtp);
      expect(_server("log").skipsEmailVerification, isFalse);
    });

    test("absent or malformed is null", () {
      expect(_server(null).emailVerification, isNull);
      expect(_server(true).emailVerification, isNull);
      expect(_server("").emailVerification, isNull);
      expect(_server(null).skipsEmailVerification, isFalse);
    });

    test("survives the stored form", () {
      final stored = PhotovaultServerInfo.fromStoredJson(
        _server("skip").toJson(),
      )!;
      expect(stored.skipsEmailVerification, isTrue);
    });

    test("a stock Ente server never skips", () {
      final ente = PhotovaultServerInfo.compatible("https://api.example.com");
      expect(ente.emailVerification, isNull);
      expect(ente.skipsEmailVerification, isFalse);
    });
  });

  group("shouldAutoSubmitEmailCode", () {
    final skip = _server("skip");

    test("sign-up and email-code login on a skipping server", () {
      for (final purpose in ["signup", "login"]) {
        expect(
          shouldAutoSubmitEmailCode(server: skip, purpose: purpose),
          isTrue,
          reason: purpose,
        );
      }
    });

    test("unchanged when the server sends or logs codes, or does not say", () {
      for (final server in [
        _server("smtp"),
        _server("log"),
        _server(null),
        _server("something-new"),
        PhotovaultServerInfo.compatible("https://api.example.com"),
        null,
      ]) {
        expect(
          shouldAutoSubmitEmailCode(server: server, purpose: "signup"),
          isFalse,
          reason: server?.emailVerification ?? "${server?.isPhotovault}",
        );
      }
    });

    test("unchanged for change of email, password reset and resends", () {
      expect(
        shouldAutoSubmitEmailCode(
          server: skip,
          purpose: null,
          isChangeEmail: true,
        ),
        isFalse,
      );
      expect(
        shouldAutoSubmitEmailCode(
          server: skip,
          purpose: "change",
          isChangeEmail: true,
        ),
        isFalse,
      );
      // Forgot password / recreate password send no purpose.
      expect(shouldAutoSubmitEmailCode(server: skip, purpose: null), isFalse);
      expect(shouldAutoSubmitEmailCode(server: skip, purpose: ""), isFalse);
      expect(
        shouldAutoSubmitEmailCode(
          server: skip,
          purpose: "signup",
          isResend: true,
        ),
        isFalse,
      );
    });

    test("the submitted code is six digits", () {
      expect(RegExp(r"^\d{6}$").hasMatch(kSkippedEmailCode), isTrue);
    });
  });
}
