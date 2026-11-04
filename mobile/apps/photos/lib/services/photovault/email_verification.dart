// PHOTOVAULT: email verification skip (new file).
//
// A Photovault server without SMTP announces `"emailVerification": "skip"`
// in `/photovault/info` and then accepts any 6-digit code after
// `POST /users/ott` (accounts with email 2FA still need their real code).
// For sign-up and email-code login the app then submits [kSkippedEmailCode]
// itself instead of showing the "Check your inbox" screen; if that fails it
// shows the screen as usual.
//
// Pure Dart; unit-tested in `test/photovault/email_verification_test.dart`.

import "package:photos/services/photovault/server_contract.dart";

/// The code sent when the server skips email verification.
const String kSkippedEmailCode = "000000";

/// `sendOtt` purposes whose code screen is skipped.
const Set<String> kSkippableOttPurposes = {"signup", "login"};

/// True when the code screen after `POST /users/ott` should be replaced by
/// an automatic [kSkippedEmailCode] submission: the active [server] is a
/// Photovault server that skips email verification, and the code is for
/// sign-up or email-code login (not a change of email, a password reset or a
/// resend).
bool shouldAutoSubmitEmailCode({
  required PhotovaultServerInfo? server,
  required String? purpose,
  bool isChangeEmail = false,
  bool isResend = false,
}) {
  if (server == null || !server.skipsEmailVerification) return false;
  if (isChangeEmail || isResend) return false;
  return purpose != null && kSkippableOttPurposes.contains(purpose);
}
