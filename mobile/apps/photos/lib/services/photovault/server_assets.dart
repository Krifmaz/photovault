// PHOTOVAULT: server connection (new file).
//
// Where the app fetches Ente's public assets (ML models, memories music) and
// which Google Cast receiver it launches. A Photovault server mirrors the
// model and music files (contracts/asset-mirror.md) and can name its own Cast
// receiver; this file turns `/photovault/info` into the URLs the app uses.
//
// Rule: use what the server advertises. Ente's hosts are used only when the
// server does not advertise a mirror (mirror turned off, or a stock server),
// so machine learning keeps working exactly as upstream.

import "package:photos/services/photovault/photovault_server_service.dart";
import "package:photos/services/photovault/server_address.dart";
import "package:photos/services/photovault/server_contract.dart";

class PhotovaultServerAssets {
  PhotovaultServerAssets._();

  /// Ente's memories music manifest.
  static const enteMusicManifestUrl = "https://music.ente.com/tracks.json";

  /// Build-time Cast receiver (`--dart-define=PHOTOVAULT_CAST_APP_ID=...`),
  /// used when the server does not announce one.
  static const _builtInCastAppId = String.fromEnvironment(
    "PHOTOVAULT_CAST_APP_ID",
  );

  static PhotovaultServerInfo? get _server =>
      PhotovaultServerService.instance.activeServer.value;

  /// False in isolates and before "Connect".
  static bool get hasServer => _server != null;

  static String get _currentAddress =>
      PhotovaultServerService.instance.currentAddress;

  /// Base URL for model files, or null for Ente's own host.
  static String? modelsMirrorUrl({
    PhotovaultServerInfo? server,
    String? currentAddress,
  }) {
    server ??= _server;
    final url = server?.modelsUrl;
    if (server == null || url == null) return null;
    return rebase(url, server, currentAddress ?? _currentAddress);
  }

  /// The memories music manifest: the server's mirror, else Ente's.
  static String musicManifestUrl({
    PhotovaultServerInfo? server,
    String? currentAddress,
  }) {
    server ??= _server;
    final url = server?.musicManifestUrl;
    if (server == null || url == null) return enteMusicManifestUrl;
    return rebase(url, server, currentAddress ?? _currentAddress);
  }

  /// The URL to download [url] from: a track on the server's public address
  /// is fetched through the address in use (home or public).
  static String downloadUrl(String url) {
    final server = _server;
    return server == null ? url : rebase(url, server, _currentAddress);
  }

  /// True when [url] is on one of the active server's own addresses. Music
  /// tracks there may be plain `http` (a LAN-only server).
  static bool isOnServer(String url, {PhotovaultServerInfo? server}) {
    server ??= _server;
    if (server == null) return false;
    final origin = _origin(url);
    if (origin == null) return false;
    return origin == _origin(server.publicUrl) ||
        (server.homeUrl != null && origin == _origin(server.homeUrl!)) ||
        origin == _origin(_currentAddress);
  }

  /// The Google Cast receiver app id to launch, or null when Chromecast
  /// auto-pairing is unavailable. Never Ente's receiver: it loads Ente's
  /// hosted cast page, which talks to Ente's servers, not this one.
  static String? castAppId() {
    final announced = _server?.castAppId;
    if (announced != null) return announced;
    final builtIn = _builtInCastAppId.trim().toUpperCase();
    return RegExp(r"^[0-9A-F]{8}$").hasMatch(builtIn) ? builtIn : null;
  }

  /// Swaps the server's public base in [url] for [currentAddress] (the LAN
  /// home address when at home), so a public address that is unreachable
  /// from inside the network does not break downloads.
  static String rebase(
    String url,
    PhotovaultServerInfo server,
    String currentAddress,
  ) {
    final public = tryNormalizeServerAddress(server.publicUrl);
    final current = tryNormalizeServerAddress(currentAddress);
    if (public == null || current == null || public == current) return url;
    if (url == public) return current;
    if (url.startsWith("$public/")) {
      return current + url.substring(public.length);
    }
    return url;
  }

  static String? _origin(String url) {
    final uri = Uri.tryParse(url);
    if (uri == null || uri.host.isEmpty) return null;
    return "${uri.host.toLowerCase()}:${uri.hasPort ? uri.port : (uri.scheme == "https" ? 443 : 80)}";
  }
}
