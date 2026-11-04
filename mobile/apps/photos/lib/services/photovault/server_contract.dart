// PHOTOVAULT: server connection (new file).
//
// The one place that mirrors the Photovault server contract
// (`contracts/server-info.md`, `contracts/apps.json`): the `/photovault/info`
// model and parser, the probe timeout, the QR payload and the mDNS service
// type. When the contract changes, change this file only.
//
// Pure Dart (no Flutter imports) so it is unit-tested in
// `test/photovault/`.

import "dart:convert";

/// Public, unauthenticated server description.
const String kPhotovaultInfoPath = "/photovault/info";

/// Ente's own health check. A stock Ente server answers here but not at
/// [kPhotovaultInfoPath]; such a server is accepted as "compatible".
const String kPingPath = "/ping";

/// DNS-SD service type the server announces on the LAN.
const String kPhotovaultMdnsService = "_photovault._tcp";

/// How long the home (LAN) address may take to answer before the public
/// address is used instead (contract: 1.5 s).
const Duration kHomeProbeTimeout = Duration(milliseconds: 1500);

/// Timeout for the "Connect" button's request.
const Duration kConnectTimeout = Duration(seconds: 10);

/// QR payload scheme: `photovault://connect?url=<publicUrl>&home=<homeUrl>&name=<name>`.
const String kConnectQrScheme = "photovault";
const String kConnectQrHost = "connect";

/// What `/photovault/info` (or, for a stock server, `/ping`) told us.
///
/// Field names follow the contract (camelCase); snake_case spellings from the
/// first draft (`home_url`, `public_url`) are accepted too.
class PhotovaultServerInfo {
  const PhotovaultServerInfo({
    required this.name,
    required this.version,
    required this.publicUrl,
    this.homeUrl,
    this.signupsAllowed = true,
    this.isPhotovault = true,
    this.commit,
    this.apiVersion,
    this.albumsUrl,
    this.castUrl,
    this.features = const [],
  });

  /// A server that answers Ente's `/ping` but has no `/photovault/info`:
  /// no name, version or home address; [address] is used as is.
  factory PhotovaultServerInfo.compatible(String address) {
    return PhotovaultServerInfo(
      name: "",
      version: "",
      publicUrl: address,
      isPhotovault: false,
    );
  }

  final String name;
  final String version;

  /// Public base URL of the API (no trailing slash).
  final String publicUrl;

  /// LAN base URL, if the server has one.
  final String? homeUrl;

  /// False hides "Create account" (the server rejects sign-ups).
  final bool signupsAllowed;

  /// False for a stock Ente-compatible server found through `/ping`.
  final bool isPhotovault;
  final String? commit;
  final int? apiVersion;
  final String? albumsUrl;
  final String? castUrl;
  final List<String> features;

  /// Parses a decoded `/photovault/info` body. Returns null when the body is
  /// not a Photovault description. Missing optional fields are tolerated;
  /// unknown fields are ignored. [fallbackUrl] (the address that was asked)
  /// stands in for a missing or empty public URL.
  static PhotovaultServerInfo? fromJson(Object? json, {String? fallbackUrl}) {
    if (json is! Map) return null;
    final product = json["product"];
    final name = _string(json["name"]);
    final publicUrl = _string(json["publicUrl"]) ?? _string(json["public_url"]);
    if (product != null) {
      if (product is! String || product.toLowerCase() != "photovault") {
        return null;
      }
    } else if (name == null || publicUrl == null) {
      // Draft servers without "product" must at least name themselves and
      // give a public URL.
      return null;
    }
    final resolvedPublic = _trimSlash(publicUrl ?? fallbackUrl);
    if (resolvedPublic == null) return null;
    final home = _trimSlash(
      _string(json["homeUrl"]) ?? _string(json["home_url"]),
    );
    final signups = json["signupsAllowed"] ?? json["signups_allowed"];
    final apiVersion = json["apiVersion"];
    final features = json["features"];
    return PhotovaultServerInfo(
      name: name ?? "Photovault",
      version: _string(json["version"]) ?? "",
      publicUrl: resolvedPublic,
      homeUrl: home,
      signupsAllowed: signups is bool ? signups : true,
      commit: _string(json["commit"]),
      apiVersion: apiVersion is int ? apiVersion : null,
      albumsUrl: _trimSlash(_string(json["albumsUrl"])),
      castUrl: _trimSlash(_string(json["castUrl"])),
      features: features is List
          ? features.whereType<String>().toList(growable: false)
          : const [],
    );
  }

  /// [fromJson] for a raw response body. Returns null for non-JSON bodies.
  static PhotovaultServerInfo? fromBody(String body, {String? fallbackUrl}) {
    try {
      return fromJson(jsonDecode(body), fallbackUrl: fallbackUrl);
    } on FormatException {
      return null;
    }
  }

  /// True when [body] is Ente's `/ping` answer (`{"message": "pong", ...}`).
  static bool isPingBody(String body) {
    try {
      final json = jsonDecode(body);
      return json is Map && json["message"] == "pong";
    } on FormatException {
      return false;
    }
  }

  /// Persisted form (camelCase, same as the wire format).
  Map<String, Object?> toJson() => {
    "product": isPhotovault ? "photovault" : "compatible",
    "name": name,
    "version": version,
    "publicUrl": publicUrl,
    "homeUrl": homeUrl,
    "signupsAllowed": signupsAllowed,
    "commit": commit,
    "apiVersion": apiVersion,
    "albumsUrl": albumsUrl,
    "castUrl": castUrl,
    "features": features,
  };

  /// Reads [toJson] back.
  static PhotovaultServerInfo? fromStoredJson(Object? json) {
    if (json is Map && json["product"] == "compatible") {
      final url = _trimSlash(_string(json["publicUrl"]));
      return url == null ? null : PhotovaultServerInfo.compatible(url);
    }
    return fromJson(json);
  }

  static String? _string(Object? value) {
    if (value is! String) return null;
    final trimmed = value.trim();
    return trimmed.isEmpty ? null : trimmed;
  }

  static String? _trimSlash(String? url) {
    if (url == null) return null;
    var out = url;
    while (out.endsWith("/")) {
      out = out.substring(0, out.length - 1);
    }
    return out.isEmpty ? null : out;
  }
}

/// The `photovault://connect?url=...&home=...&name=...` link for [server]
/// (the same payload as the server's QR code; [parseConnectPayload] reads it).
String buildConnectLink(PhotovaultServerInfo server) {
  return Uri(
    scheme: kConnectQrScheme,
    host: kConnectQrHost,
    queryParameters: {
      "url": server.publicUrl,
      if (server.homeUrl != null && server.homeUrl!.isNotEmpty)
        "home": server.homeUrl!,
      if (server.isPhotovault && server.name.isNotEmpty) "name": server.name,
    },
  ).toString();
}

/// A server address read from a QR code or a pasted link.
class ConnectPayload {
  const ConnectPayload({required this.url, this.homeUrl, this.name});

  /// The address to connect to (public URL, or whatever was scanned).
  final String url;
  final String? homeUrl;
  final String? name;
}

/// Reads a QR code / pasted text. Accepts
/// `photovault://connect?url=...&home=...&name=...`, a bare `http(s)://`
/// URL (also one ending in `/photovault/info`), or a bare `host:port`.
/// Returns null when nothing usable is found. The returned URL is not yet
/// normalised (see `server_address.dart`).
ConnectPayload? parseConnectPayload(String raw) {
  final text = raw.trim();
  if (text.isEmpty) return null;
  final lower = text.toLowerCase();
  if (lower.startsWith("$kConnectQrScheme:")) {
    final uri = Uri.tryParse(text);
    if (uri == null) return null;
    String? param(String key) {
      final value = uri.queryParameters[key]?.trim();
      return (value == null || value.isEmpty) ? null : value;
    }

    final url = param("url") ?? param("public") ?? param("home");
    if (url == null) return null;
    return ConnectPayload(
      url: url,
      homeUrl: param("home"),
      name: param("name"),
    );
  }
  // Only the first token: some QR generators append text after the URL.
  final first = text.split(RegExp(r"\s+")).first;
  if (first.isEmpty) return null;
  return ConnectPayload(url: first);
}

/// A server announced on the LAN through [kPhotovaultMdnsService].
class DiscoveredServer {
  const DiscoveredServer({
    required this.name,
    required this.address,
    this.version,
  });

  final String name;

  /// Address to connect to (not yet normalised).
  final String address;
  final String? version;
}

/// Builds a [DiscoveredServer] from an mDNS answer. TXT keys (contract):
/// `name`, `version`, `public`, `home`, `info`. The LAN address the answer
/// came from ([ip], [port]) is preferred, then `home`, then `public`.
DiscoveredServer? discoveredServerFromMdns({
  required String instanceName,
  required Map<String, String> txt,
  String? ip,
  int? port,
}) {
  String? value(String key) {
    final v = txt[key]?.trim();
    return (v == null || v.isEmpty) ? null : v;
  }

  String? address;
  if (ip != null && ip.isNotEmpty && port != null && port > 0) {
    final host = ip.contains(":") ? "[$ip]" : ip;
    address = "http://$host:$port";
  }
  address ??= value("home") ?? value("public");
  if (address == null) return null;
  final name =
      value("name") ??
      instanceName.split(".$kPhotovaultMdnsService").first.trim();
  return DiscoveredServer(
    name: name.isEmpty ? "Photovault" : name,
    address: address,
    version: value("version"),
  );
}

/// True when [uri] is a public (or collect) album link made by [server]
/// (`contracts/apps.json` shareLink: `<albumsUrl>/?t=<token>#<key>`, where
/// albumsUrl defaults to `publicUrl/albums`). The host must be the host of
/// the server's albums, public or home URL, and the link must carry a token.
bool isServerPublicAlbumLink(Uri? uri, PhotovaultServerInfo? server) {
  if (uri == null || server == null) return false;
  final scheme = uri.scheme.toLowerCase();
  if (scheme != "https" && scheme != "http") return false;
  final token = uri.queryParameters["t"];
  if (token == null || token.isEmpty) return false;
  final host = uri.host.toLowerCase();
  if (host.isEmpty) return false;
  for (final base in [server.albumsUrl, server.publicUrl, server.homeUrl]) {
    if (base == null) continue;
    final baseHost = Uri.tryParse(base)?.host.toLowerCase();
    if (baseHost != null && baseHost.isNotEmpty && baseHost == host) {
      return true;
    }
  }
  return false;
}

/// Base URL of the server's public-album web app (`contracts/apps.json`
/// shareLink: `<albumsUrl>/?t=<token>#<key>`): the announced `albumsUrl`,
/// else `<publicUrl>/albums`, else `<endpoint>/albums` for a server that does
/// not describe itself. Never an Ente host (upstream used Ente's albums host).
String serverAlbumsBaseUrl(PhotovaultServerInfo? server, String endpoint) {
  final announced = server?.albumsUrl;
  if (announced != null && announced.isNotEmpty) return announced;
  if (server != null && server.isPhotovault && server.publicUrl.isNotEmpty) {
    return "${server.publicUrl}/albums";
  }
  var base = endpoint;
  while (base.endsWith("/")) {
    base = base.substring(0, base.length - 1);
  }
  return "$base/albums";
}
