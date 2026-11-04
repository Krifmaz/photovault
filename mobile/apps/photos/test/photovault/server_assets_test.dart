// PHOTOVAULT: server connection (new file). Model/music mirror and Cast
// receiver come from /photovault/info (contracts/asset-mirror.md).
import "package:flutter_test/flutter_test.dart";
import "package:photos/services/photovault/server_assets.dart";
import "package:photos/services/photovault/server_contract.dart";

PhotovaultServerInfo _info(Map<String, Object?> extra) =>
    PhotovaultServerInfo.fromJson({
      "product": "photovault",
      "name": "Home",
      "version": "0.2.0",
      "publicUrl": "https://photos.example.com",
      "homeUrl": "http://10.0.1.7:8095",
      ...extra,
    })!;

void main() {
  final mirrored = _info({
    "modelsUrl": "https://photos.example.com/photovault/models/",
    "musicManifestUrl":
        "https://photos.example.com/photovault/music/tracks.json",
    "castAppId": "1a2b3c4d",
  });
  final noMirror = _info({"modelsUrl": null, "musicManifestUrl": null});

  test("parses the mirror and Cast fields and keeps them when stored", () {
    expect(mirrored.modelsUrl, "https://photos.example.com/photovault/models");
    expect(mirrored.castAppId, "1A2B3C4D");
    final stored = PhotovaultServerInfo.fromStoredJson(mirrored.toJson())!;
    expect(stored.modelsUrl, mirrored.modelsUrl);
    expect(stored.musicManifestUrl, mirrored.musicManifestUrl);
    expect(stored.castAppId, "1A2B3C4D");
    expect(_info({"castAppId": "https://x"}).castAppId, isNull);
  });

  test("uses the server's mirror through the address in use", () {
    expect(
      PhotovaultServerAssets.modelsMirrorUrl(
        server: mirrored,
        currentAddress: "https://photos.example.com",
      ),
      "https://photos.example.com/photovault/models",
    );
    expect(
      PhotovaultServerAssets.modelsMirrorUrl(
        server: mirrored,
        currentAddress: "http://10.0.1.7:8095",
      ),
      "http://10.0.1.7:8095/photovault/models",
    );
    expect(
      PhotovaultServerAssets.musicManifestUrl(
        server: mirrored,
        currentAddress: "http://10.0.1.7:8095",
      ),
      "http://10.0.1.7:8095/photovault/music/tracks.json",
    );
  });

  test("falls back to Ente's hosts only without a mirror", () {
    expect(
      PhotovaultServerAssets.modelsMirrorUrl(
        server: noMirror,
        currentAddress: "https://photos.example.com",
      ),
      isNull,
    );
    expect(
      PhotovaultServerAssets.musicManifestUrl(
        server: noMirror,
        currentAddress: "https://photos.example.com",
      ),
      PhotovaultServerAssets.enteMusicManifestUrl,
    );
  });

  test("rebase leaves other hosts alone", () {
    expect(
      PhotovaultServerAssets.rebase(
        "https://photos.example.community/a.mp3",
        mirrored,
        "http://10.0.1.7:8095",
      ),
      "https://photos.example.community/a.mp3",
    );
  });

  test("tracks on the server's own addresses may be plain http", () {
    expect(
      PhotovaultServerAssets.isOnServer(
        "http://10.0.1.7:8095/photovault/music/a.mp3",
        server: mirrored,
      ),
      isTrue,
    );
    expect(
      PhotovaultServerAssets.isOnServer(
        "http://photos.example.com/photovault/music/a.mp3",
        server: mirrored,
      ),
      isFalse, // https origin is port 443, not 80
    );
    expect(
      PhotovaultServerAssets.isOnServer(
        "http://evil.example.com/a.mp3",
        server: mirrored,
      ),
      isFalse,
    );
  });

  test("no Cast receiver without a server or build-time id", () {
    expect(PhotovaultServerAssets.castAppId(), isNull);
  });
}
