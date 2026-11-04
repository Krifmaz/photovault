// PHOTOVAULT: server connection (new file).
//
// English text for the server-connection screens. Kept out of the shared
// ARB files so upstream string merges stay conflict-free; move into
// `packages/strings` when these screens get translated.

import "package:photos/services/photovault/server_address.dart";
import "package:photos/services/photovault/server_contract.dart";
import "package:photos/services/photovault/server_info_client.dart";

class PhotovaultStrings {
  const PhotovaultStrings._();

  static const connectTitle = "Connect to your server";
  static const connectSubtitle =
      "Enter the address of your Photovault server, scan its QR code, or pick it from your network.";
  static const addressLabel = "Server address";
  static const addressHint = "photos.example.com or 10.0.1.7:8095";
  static const connect = "Connect";
  static const connecting = "Connecting…";
  static const continueLabel = "Continue";
  static const scanQr = "Scan QR code";
  static const scanQrHint =
      "Point the camera at the QR code in your server's web settings.";
  static const qrNotAServer = "This QR code doesn't contain a server address.";
  static const savedServers = "Your servers";
  static const onThisNetwork = "On this network";
  static const searching = "Looking for servers…";
  static const noneFound = "No servers found on this network.";
  static const searchAgain = "Search again";
  static const builtIn = "Built-in server";
  static const forget = "Remove";
  static const useWithoutServer = "Use without a server";
  static const compatibleServer = "Compatible server";
  static const compatibleServerNote =
      "This server doesn't describe itself, so its name and version are unknown.";
  static const server = "Server";
  static const changeServer = "Change server";
  static const switchServer = "Switch server";
  static const notConnected = "Not connected";
  static const mlModelsUnavailable =
      "Unavailable: this server doesn't provide the models";
  static const address = "Address";
  static const homeAddress = "Home address";
  static const publicAddress = "Public address";
  static const version = "Version";
  static const inUse = "in use";
  static const signupsClosed =
      "This server doesn't accept new accounts. Ask its administrator for one.";

  static String connectedTo(String name) => "Connected to $name";

  /// Invite texts (sharing, family) end with the server to sign up on: its
  /// public address and the connect link. Unchanged without a server.
  static String inviteWithServer(String text, PhotovaultServerInfo? server) {
    if (server == null) return text;
    return "$text\n\nServer: ${server.publicUrl}\n${buildConnectLink(server)}";
  }

  static String connectLinkWhileSignedIn(String name) =>
      "You're signed in${name.isEmpty ? "" : " to $name"}. To use another server, open Settings > Server and switch server.";

  static String switchServerBody(String name) =>
      "You'll be logged out of $name first. Your photos stay on the server.";

  static String versionLabel(String version) =>
      version.isEmpty ? "" : "Version $version";

  static String connectError(ServerConnectException e) {
    final address = e.address;
    switch (e.kind) {
      case ServerConnectErrorKind.invalidAddress:
        return "\"$address\" isn't a server address. Try something like photos.example.com or 10.0.1.7:8095.";
      case ServerConnectErrorKind.unreachable:
        // The home Wi-Fi hint only fits a LAN-only address.
        return isHomeNetworkAddress(address)
            ? "Can't reach $address. Are you on your home Wi-Fi?"
            : "Can't reach $address. Check the address and your internet connection.";
      case ServerConnectErrorKind.tls:
        return "Couldn't make a secure connection to $address. Its certificate isn't trusted, or it doesn't speak HTTPS. For a home address, try http://$address.";
      case ServerConnectErrorKind.notPhotovault:
        return "$address answered, but it isn't a Photovault server. Check the address and port.";
      case ServerConnectErrorKind.httpError:
        return "$address answered with an error (HTTP ${e.statusCode ?? "?"}). Try again in a moment.";
    }
  }
}
