// PHOTOVAULT: server connection tests (pure Dart).
import "package:flutter_test/flutter_test.dart";
import "package:photos/services/photovault/photovault_strings.dart";
import "package:photos/services/photovault/server_contract.dart";
import "package:photos/services/photovault/server_info_client.dart";

void main() {
  test("unreachable: home Wi-Fi hint only for LAN addresses", () {
    final lan = PhotovaultStrings.connectError(
      const ServerConnectException(
        ServerConnectErrorKind.unreachable,
        "192.168.1.10:8080",
      ),
    );
    expect(lan, contains("home Wi-Fi"));
    final public = PhotovaultStrings.connectError(
      const ServerConnectException(
        ServerConnectErrorKind.unreachable,
        "photos.example.com",
      ),
    );
    expect(public, isNot(contains("Wi-Fi")));
    expect(public, contains("internet connection"));
  });

  test("invite text carries the server address and connect link", () {
    const server = PhotovaultServerInfo(
      name: "Family",
      version: "0.2.0",
      publicUrl: "https://photos.example.com",
    );
    final text = PhotovaultStrings.inviteWithServer(
      "Download Photovault",
      server,
    );
    expect(text, startsWith("Download Photovault\n\n"));
    expect(text, contains("https://photos.example.com"));
    expect(text, contains("photovault://connect?url="));
    expect(PhotovaultStrings.inviteWithServer("x", null), "x");
  });
}
