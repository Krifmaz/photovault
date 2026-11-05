// PHOTOVAULT: server connection tests (pure Dart, mock HTTP).
import "dart:async";
import "dart:convert";
import "dart:io";

import "package:flutter_test/flutter_test.dart";
import "package:http/http.dart" as http;
import "package:http/testing.dart";
import "package:photos/services/photovault/server_info_client.dart";

PhotovaultServerInfoClient _client(
  Future<http.Response> Function(http.Request) handler,
) {
  return PhotovaultServerInfoClient(clientFactory: () => MockClient(handler));
}

Future<ServerConnectErrorKind> _kind(Future<Object?> f) async {
  try {
    await f;
  } on ServerConnectException catch (e) {
    return e.kind;
  }
  fail("expected ServerConnectException");
}

void main() {
  const base = "http://192.168.1.10:8080";

  test("reads /photovault/info", () async {
    final client = _client((req) async {
      expect(req.url.toString(), "$base/photovault/info");
      return http.Response(
        jsonEncode({
          "product": "photovault",
          "name": "Family",
          "version": "0.2.0",
          "publicUrl": "https://photos.example.com",
          "homeUrl": base,
          "signupsAllowed": true,
        }),
        200,
      );
    });
    final info = await client.fetch(base);
    expect(info.name, "Family");
    expect(info.homeUrl, base);
  });

  test("falls back to /ping for an Ente-compatible server", () async {
    final client = _client((req) async {
      if (req.url.path == "/ping") {
        return http.Response('{"id":"1","message":"pong"}', 200);
      }
      return http.Response("not found", 404);
    });
    final info = await client.fetch(base);
    expect(info.isPhotovault, isFalse);
    expect(info.publicUrl, base);
  });

  test("not a Photovault server", () async {
    final client = _client((req) async => http.Response("<html></html>", 200));
    expect(
      await _kind(client.fetch(base)),
      ServerConnectErrorKind.notPhotovault,
    );
  });

  test("server error", () async {
    final client = _client((req) async => http.Response("oops", 502));
    try {
      await client.fetch(base);
      fail("expected error");
    } on ServerConnectException catch (e) {
      expect(e.kind, ServerConnectErrorKind.httpError);
      expect(e.statusCode, 502);
      expect(e.address, "192.168.1.10:8080");
    }
  });

  test("unreachable and TLS errors are told apart", () async {
    expect(
      await _kind(
        _client(
          (req) async => throw const SocketException("Connection refused"),
        ).fetch(base),
      ),
      ServerConnectErrorKind.unreachable,
    );
    expect(
      await _kind(
        _client(
          (req) async => throw http.ClientException("Connection refused"),
        ).fetch(base),
      ),
      ServerConnectErrorKind.unreachable,
    );
    expect(
      await _kind(
        _client(
          (req) async =>
              throw const HandshakeException("CERTIFICATE_VERIFY_FAILED"),
        ).fetch("https://photos.example.com"),
      ),
      ServerConnectErrorKind.tls,
    );
    expect(
      await _kind(
        _client(
          (req) => Completer<http.Response>().future,
        ).fetch(base, timeout: const Duration(milliseconds: 10)),
      ),
      ServerConnectErrorKind.unreachable,
    );
  });

  test("probe never throws", () async {
    final client = _client((req) async => throw const SocketException("x"));
    expect(await client.probe(base, const Duration(seconds: 1)), isNull);
  });
}
