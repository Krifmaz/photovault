// PHOTOVAULT: server connection (new file).
//
// Turns what a person types (or scans) into the base URL the app talks to.
// Pure Dart; unit-tested in `test/photovault/server_address_test.dart`.

import "package:photos/services/photovault/server_contract.dart";

class InvalidServerAddressException implements Exception {
  const InvalidServerAddressException(this.input);

  final String input;

  @override
  String toString() => "InvalidServerAddressException($input)";
}

/// Normalises a server address:
///
/// - trims whitespace; accepts `host`, `host:port`, `[v6]:port`, `http://`
///   and `https://` URLs, and `photovault://connect?url=...` links;
/// - without a scheme, uses `http` for addresses that only make sense on a
///   home network (IP literals, `localhost`, single-label names, `.local`,
///   `.lan`, `.home`, `.internal`, `.home.arpa`) and `https` otherwise;
/// - lower-cases scheme and host, drops default ports, user info, query and
///   fragment, trailing slashes and a trailing `/photovault/info` or `/ping`;
/// - keeps any other path (a server behind a reverse-proxy sub-path).
///
/// Throws [InvalidServerAddressException] for anything else.
String normalizeServerAddress(String input) {
  var text = input.trim();
  if (text.toLowerCase().startsWith("$kConnectQrScheme:")) {
    final payload = parseConnectPayload(text);
    if (payload == null) throw InvalidServerAddressException(input);
    text = payload.url.trim();
  }
  if (text.isEmpty || text.contains(RegExp(r"\s"))) {
    throw InvalidServerAddressException(input);
  }
  final schemeMatch = RegExp(r"^([a-zA-Z][a-zA-Z0-9+.-]*)://").firstMatch(text);
  if (schemeMatch == null) {
    text = "${_defaultSchemeFor(text)}://$text";
  }
  final Uri uri;
  try {
    uri = Uri.parse(text);
  } on FormatException {
    throw InvalidServerAddressException(input);
  }
  final scheme = uri.scheme.toLowerCase();
  if (scheme != "http" && scheme != "https") {
    throw InvalidServerAddressException(input);
  }
  final host = uri.host.toLowerCase();
  if (host.isEmpty || !_validHost(host)) {
    throw InvalidServerAddressException(input);
  }
  final hasPort =
      uri.hasPort &&
      !((scheme == "http" && uri.port == 80) ||
          (scheme == "https" && uri.port == 443));
  var path = uri.path;
  while (path.endsWith("/")) {
    path = path.substring(0, path.length - 1);
  }
  for (final suffix in [kPhotovaultInfoPath, kPingPath]) {
    if (path.toLowerCase().endsWith(suffix)) {
      path = path.substring(0, path.length - suffix.length);
    }
  }
  while (path.endsWith("/")) {
    path = path.substring(0, path.length - 1);
  }
  final hostPart = host.contains(":") ? "[$host]" : host;
  return "$scheme://$hostPart${hasPort ? ":${uri.port}" : ""}$path";
}

/// Like [normalizeServerAddress] but returns null instead of throwing.
String? tryNormalizeServerAddress(String? input) {
  if (input == null) return null;
  try {
    return normalizeServerAddress(input);
  } on InvalidServerAddressException {
    return null;
  }
}

/// Short form for messages: `10.0.1.7:8095`, `photos.example.com`.
String displayServerAddress(String url) {
  final uri = Uri.tryParse(url);
  if (uri == null || uri.host.isEmpty) return url;
  final host = uri.host.contains(":") ? "[${uri.host}]" : uri.host;
  final defaultPort =
      (uri.scheme == "http" && uri.port == 80) ||
      (uri.scheme == "https" && uri.port == 443);
  final port = uri.hasPort && !defaultPort ? ":${uri.port}" : "";
  final path = uri.path == "/" ? "" : uri.path;
  return "$host$port$path";
}

/// True when two base URLs point at the same server address.
bool sameServerAddress(String? a, String? b) {
  if (a == null || b == null) return false;
  final na = tryNormalizeServerAddress(a);
  final nb = tryNormalizeServerAddress(b);
  return na != null && na == nb;
}

/// True for addresses that only make sense on a home network (IP literals,
/// `localhost`, single-label names, `.local`, `.lan`, `.home`, `.internal`,
/// `.home.arpa`): the same rule that picks `http` as the default scheme.
bool isHomeNetworkAddress(String address) {
  var text = address.trim();
  final schemeMatch = RegExp(r"^[a-zA-Z][a-zA-Z0-9+.-]*://").firstMatch(text);
  if (schemeMatch != null) text = text.substring(schemeMatch.end);
  if (text.isEmpty) return false;
  return _defaultSchemeFor(text) == "http";
}

String _defaultSchemeFor(String text) {
  var host = text;
  if (host.startsWith("[")) return "http"; // IPv6 literal
  final cut = host.indexOf(RegExp(r"[:/?#]"));
  if (cut >= 0) host = host.substring(0, cut);
  host = host.toLowerCase();
  if (host == "localhost" || !host.contains(".")) return "http";
  if (RegExp(r"^\d{1,3}(\.\d{1,3}){3}$").hasMatch(host)) return "http";
  const localSuffixes = [".local", ".lan", ".home", ".internal", ".home.arpa"];
  if (localSuffixes.any(host.endsWith)) return "http";
  return "https";
}

bool _validHost(String host) {
  if (host.contains(":")) return true; // IPv6, already parsed by Uri
  if (RegExp(r"^\d+(\.\d+)*$").hasMatch(host)) {
    final parts = host.split(".");
    return parts.length == 4 && parts.every((p) => int.parse(p) <= 255);
  }
  return RegExp(
    r"^[a-z0-9]([a-z0-9_-]*[a-z0-9])?(\.[a-z0-9]([a-z0-9_-]*[a-z0-9])?)*\.?$",
  ).hasMatch(host);
}
