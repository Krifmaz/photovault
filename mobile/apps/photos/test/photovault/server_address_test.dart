// PHOTOVAULT: server connection tests (pure Dart).
import "package:flutter_test/flutter_test.dart";
import "package:photos/services/photovault/server_address.dart";

void main() {
  group("normalizeServerAddress", () {
    final cases = <String, String>{
      "192.168.1.10:8080": "http://192.168.1.10:8080",
      "  192.168.1.10:8080/  ": "http://192.168.1.10:8080",
      "localhost:8080": "http://localhost:8080",
      "nas:8080": "http://nas:8080",
      "nas.local:8080": "http://nas.local:8080",
      "photos.home.arpa": "http://photos.home.arpa",
      "photos.example.com": "https://photos.example.com",
      "Photos.Example.COM/": "https://photos.example.com",
      "HTTPS://photos.example.com:443/": "https://photos.example.com",
      "http://photos.example.com:80": "http://photos.example.com",
      "http://photos.example.com:443": "http://photos.example.com:443",
      "https://photos.example.com/photovault/info":
          "https://photos.example.com",
      "https://photos.example.com/vault/photovault/info/":
          "https://photos.example.com/vault",
      "https://photos.example.com/ping": "https://photos.example.com",
      "https://photos.example.com/vault?x=1#y":
          "https://photos.example.com/vault",
      "https://user:pw@photos.example.com": "https://photos.example.com",
      "[fd00::7]:8080": "http://[fd00::7]:8080",
      "http://[FD00::7]:8080/": "http://[fd00::7]:8080",
      "photovault://connect?url=https%3A%2F%2Fphotos.example.com&home=http%3A%2F%2F192.168.1.10%3A8080":
          "https://photos.example.com",
    };
    cases.forEach((input, expected) {
      test(input, () => expect(normalizeServerAddress(input), expected));
    });

    for (final bad in [
      "",
      "   ",
      "ftp://photos.example.com",
      "http://",
      "photos example.com",
      "999.1.1.1:80",
      "https://bad_host!.com",
      "photovault://connect?name=x",
    ]) {
      test("rejects '$bad'", () {
        expect(
          () => normalizeServerAddress(bad),
          throwsA(isA<InvalidServerAddressException>()),
        );
        expect(tryNormalizeServerAddress(bad), isNull);
      });
    }
  });

  test("displayServerAddress", () {
    expect(
      displayServerAddress("http://192.168.1.10:8080"),
      "192.168.1.10:8080",
    );
    expect(
      displayServerAddress("https://photos.example.com"),
      "photos.example.com",
    );
    expect(
      displayServerAddress("https://photos.example.com/vault"),
      "photos.example.com/vault",
    );
  });

  test("sameServerAddress", () {
    expect(
      sameServerAddress("https://photos.example.com/", "photos.example.com"),
      isTrue,
    );
    expect(
      sameServerAddress(
        "http://192.168.1.10:8080",
        "https://192.168.1.10:8080",
      ),
      isFalse,
    );
    expect(sameServerAddress(null, "x"), isFalse);
  });

  group("isHomeNetworkAddress", () {
    test("LAN-only addresses", () {
      for (final a in [
        "192.168.1.10:8080",
        "http://192.168.1.2",
        "nas.local",
        "photos.lan:8080",
        "localhost",
        "nas",
        "[fe80::1]:8080",
      ]) {
        expect(isHomeNetworkAddress(a), isTrue, reason: a);
      }
    });
    test("public addresses", () {
      for (final a in [
        "photos.example.com",
        "https://photos.example.com",
        "photos.example.com:8443",
      ]) {
        expect(isHomeNetworkAddress(a), isFalse, reason: a);
      }
      expect(isHomeNetworkAddress(""), isFalse);
    });
  });
}
