// PHOTOVAULT: server connection (new file).
//
// Asks a server what it is (`/photovault/info`, falling back to Ente's
// `/ping`) and turns failures into the few cases the connect screen
// explains. Pure Dart (package:http); unit-tested with a mock client.

import "dart:async";
import "dart:io";

import "package:http/http.dart" as http;
import "package:photos/services/photovault/server_address.dart";
import "package:photos/services/photovault/server_contract.dart";

enum ServerConnectErrorKind {
  /// The text is not an address.
  invalidAddress,

  /// No answer: DNS, refused, timeout, no route.
  unreachable,

  /// TLS handshake or certificate failed.
  tls,

  /// Something answered, but neither `/photovault/info` nor `/ping`.
  notPhotovault,

  /// The server answered with an HTTP error.
  httpError,
}

class ServerConnectException implements Exception {
  const ServerConnectException(
    this.kind,
    this.address, {
    this.statusCode,
    this.cause,
  });

  final ServerConnectErrorKind kind;

  /// Display form of the address (see [displayServerAddress]).
  final String address;
  final int? statusCode;
  final Object? cause;

  @override
  String toString() =>
      "ServerConnectException($kind, $address, status: $statusCode, cause: $cause)";
}

/// Maps a low-level error to an error kind.
ServerConnectErrorKind classifyConnectError(Object error) {
  if (error is ServerConnectException) return error.kind;
  if (error is HandshakeException ||
      error is TlsException ||
      error is CertificateException) {
    return ServerConnectErrorKind.tls;
  }
  if (error is SocketException ||
      error is TimeoutException ||
      error is HttpException) {
    return ServerConnectErrorKind.unreachable;
  }
  if (error is http.ClientException) {
    final message = error.message.toLowerCase();
    if (message.contains("handshake") ||
        message.contains("certificate") ||
        message.contains("tls")) {
      return ServerConnectErrorKind.tls;
    }
    return ServerConnectErrorKind.unreachable;
  }
  return ServerConnectErrorKind.unreachable;
}

class PhotovaultServerInfoClient {
  PhotovaultServerInfoClient({http.Client Function()? clientFactory})
    : _clientFactory = clientFactory ?? http.Client.new;

  final http.Client Function() _clientFactory;

  /// Fetches the description of the server at [baseUrl] (already
  /// normalised). Throws [ServerConnectException].
  Future<PhotovaultServerInfo> fetch(
    String baseUrl, {
    Duration timeout = kConnectTimeout,
  }) async {
    final display = displayServerAddress(baseUrl);
    final client = _clientFactory();
    try {
      final infoResponse = await client
          .get(Uri.parse("$baseUrl$kPhotovaultInfoPath"), headers: _headers)
          .timeout(timeout);
      if (infoResponse.statusCode == 200) {
        final info = PhotovaultServerInfo.fromBody(
          infoResponse.body,
          fallbackUrl: baseUrl,
        );
        if (info != null) return info;
      } else if (infoResponse.statusCode >= 500) {
        throw ServerConnectException(
          ServerConnectErrorKind.httpError,
          display,
          statusCode: infoResponse.statusCode,
        );
      }
      // 404 or a page that is not ours: maybe a stock Ente-compatible server.
      final pingResponse = await client
          .get(Uri.parse("$baseUrl$kPingPath"), headers: _headers)
          .timeout(timeout);
      if (pingResponse.statusCode == 200 &&
          PhotovaultServerInfo.isPingBody(pingResponse.body)) {
        return PhotovaultServerInfo.compatible(baseUrl);
      }
      throw ServerConnectException(
        ServerConnectErrorKind.notPhotovault,
        display,
        statusCode: infoResponse.statusCode,
      );
    } on ServerConnectException {
      rethrow;
    } catch (e) {
      throw ServerConnectException(classifyConnectError(e), display, cause: e);
    } finally {
      client.close();
    }
  }

  /// Like [fetch] but never throws: null when the server does not answer
  /// in time or is not a Photovault server. Used by the home/public probe.
  Future<PhotovaultServerInfo?> probe(String baseUrl, Duration timeout) async {
    final client = _clientFactory();
    try {
      final response = await client
          .get(Uri.parse("$baseUrl$kPhotovaultInfoPath"), headers: _headers)
          .timeout(timeout);
      if (response.statusCode != 200) return null;
      return PhotovaultServerInfo.fromBody(response.body);
    } catch (_) {
      return null;
    } finally {
      client.close();
    }
  }

  static const _headers = {"Accept": "application/json"};
}
