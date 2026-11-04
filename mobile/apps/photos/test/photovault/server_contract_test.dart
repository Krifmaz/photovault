// PHOTOVAULT: server connection tests (pure Dart).
import "dart:convert";

import "package:flutter_test/flutter_test.dart";
import "package:photos/services/photovault/server_contract.dart";

void main() {
  group("PhotovaultServerInfo.fromJson", () {
    test("contract example", () {
      final info = PhotovaultServerInfo.fromBody(
        jsonEncode({
          "product": "photovault",
          "name": "Family photos",
          "version": "0.2.0",
          "commit": "f145822",
          "apiVersion": 1,
          "publicUrl": "https://photos.example.com/",
          "homeUrl": "http://10.0.1.7:8095",
          "albumsUrl": "https://photos.example.com/albums",
          "castUrl": "https://photos.example.com/cast",
          "signupsAllowed": false,
          "emailVerification": "smtp",
          "storage": "local",
          "features": ["web", "mdns", 3],
          "somethingNew": {"ignored": true},
        }),
      )!;
      expect(info.isPhotovault, isTrue);
      expect(info.name, "Family photos");
      expect(info.version, "0.2.0");
      expect(info.publicUrl, "https://photos.example.com");
      expect(info.homeUrl, "http://10.0.1.7:8095");
      expect(info.signupsAllowed, isFalse);
      expect(info.apiVersion, 1);
      expect(info.features, ["web", "mdns"]);
    });

    test("snake_case draft fields without product", () {
      final info = PhotovaultServerInfo.fromJson({
        "name": "Photovault",
        "version": "0.1.0",
        "home_url": null,
        "public_url": "https://p.example.com",
      })!;
      expect(info.publicUrl, "https://p.example.com");
      expect(info.homeUrl, isNull);
      expect(info.signupsAllowed, isTrue);
    });

    test("missing optional fields and empty public URL use the fallback", () {
      final info = PhotovaultServerInfo.fromJson({
        "product": "photovault",
        "name": "",
        "publicUrl": "",
      }, fallbackUrl: "http://10.0.1.7:8095")!;
      expect(info.name, "Photovault");
      expect(info.version, "");
      expect(info.publicUrl, "http://10.0.1.7:8095");
    });

    test("rejects other products and non-objects", () {
      expect(
        PhotovaultServerInfo.fromJson({"product": "museum", "name": "x"}),
        isNull,
      );
      expect(PhotovaultServerInfo.fromJson({"name": "x"}), isNull);
      expect(PhotovaultServerInfo.fromJson([1, 2]), isNull);
      expect(PhotovaultServerInfo.fromBody("<html>"), isNull);
    });

    test("round-trips through storage", () {
      final info = PhotovaultServerInfo.fromJson({
        "product": "photovault",
        "name": "A",
        "version": "1",
        "publicUrl": "https://a.example.com",
        "homeUrl": "http://10.0.0.2:8095",
        "signupsAllowed": false,
      })!;
      final back = PhotovaultServerInfo.fromStoredJson(
        jsonDecode(jsonEncode(info.toJson())),
      )!;
      expect(back.name, "A");
      expect(back.homeUrl, "http://10.0.0.2:8095");
      expect(back.signupsAllowed, isFalse);

      final compat = PhotovaultServerInfo.fromStoredJson(
        jsonDecode(
          jsonEncode(
            PhotovaultServerInfo.compatible("http://10.0.0.3:8080").toJson(),
          ),
        ),
      )!;
      expect(compat.isPhotovault, isFalse);
      expect(compat.publicUrl, "http://10.0.0.3:8080");
    });
  });

  test("isPingBody", () {
    expect(
      PhotovaultServerInfo.isPingBody('{"id":"x","message":"pong"}'),
      isTrue,
    );
    expect(PhotovaultServerInfo.isPingBody('{"message":"no"}'), isFalse);
    expect(PhotovaultServerInfo.isPingBody("pong"), isFalse);
  });

  group("parseConnectPayload", () {
    test("photovault:// link", () {
      final p = parseConnectPayload(
        "photovault://connect?url=https%3A%2F%2Fphotos.example.com"
        "&home=http%3A%2F%2F10.0.1.7%3A8095&name=Family%20photos",
      )!;
      expect(p.url, "https://photos.example.com");
      expect(p.homeUrl, "http://10.0.1.7:8095");
      expect(p.name, "Family photos");
    });
    test("bare URL and trailing text", () {
      expect(
        parseConnectPayload(
          " https://photos.example.com/photovault/info ",
        )!.url,
        "https://photos.example.com/photovault/info",
      );
      expect(
        parseConnectPayload("10.0.1.7:8095 Photovault")!.url,
        "10.0.1.7:8095",
      );
    });
    test("home only, and nothing usable", () {
      expect(
        parseConnectPayload("PHOTOVAULT://connect?home=10.0.1.7:8095")!.url,
        "10.0.1.7:8095",
      );
      expect(parseConnectPayload("photovault://connect"), isNull);
      expect(parseConnectPayload("   "), isNull);
    });
  });

  group("discoveredServerFromMdns", () {
    test("prefers the answering LAN address", () {
      final s = discoveredServerFromMdns(
        instanceName: "Family._photovault._tcp.local",
        txt: {"name": "Family photos", "version": "0.2.0", "home": "x"},
        ip: "10.0.1.7",
        port: 8095,
      )!;
      expect(s.address, "http://10.0.1.7:8095");
      expect(s.name, "Family photos");
      expect(s.version, "0.2.0");
    });
    test("falls back to TXT home, then public, and the instance name", () {
      final s = discoveredServerFromMdns(
        instanceName: "NAS._photovault._tcp.local",
        txt: {"public": "https://p.example.com"},
      )!;
      expect(s.address, "https://p.example.com");
      expect(s.name, "NAS");
      expect(
        discoveredServerFromMdns(instanceName: "x", txt: const {}),
        isNull,
      );
    });
  });
}
