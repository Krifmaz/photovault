# `GET /photovault/info`

Public, no auth, CORS-open (`Access-Control-Allow-Origin: *`), `Cache-Control: no-store`.
Used by every app's "Connect to your server" screen, the mDNS flow and the QR page.
A client treats a server as Photovault when this returns 200 with `"product": "photovault"`.
A stock Ente museum returns 404 here; clients then fall back to `GET /ping` and show
"Ente-compatible server" with no name/version.

```json
{
  "product": "photovault",
  "name": "Photovault",
  "version": "0.2.0",
  "commit": "f145822",
  "apiVersion": 1,
  "publicUrl": "https://photos.example.com",
  "homeUrl": "http://10.0.1.7:8095",
  "albumsUrl": "https://photos.example.com/albums",
  "castUrl": "https://photos.example.com/cast",
  "castAppId": "1A2B3C4D",
  "modelsUrl": "https://photos.example.com/photovault/models",
  "musicManifestUrl": "https://photos.example.com/photovault/music/tracks.json",
  "signupsAllowed": false,
  "emailVerification": "smtp",
  "storage": "local",
  "features": ["admin-cli", "web", "public-albums", "cast", "mdns", "models", "music"]
}
```

| Field | Type | Notes |
| --- | --- | --- |
| `product` | `"photovault"` | constant |
| `name` | string | `PHOTOVAULT_SERVER_NAME`, default `"Photovault"`; shown in apps |
| `version` | string | semver of the server build |
| `commit` | string \| null | short git hash |
| `apiVersion` | int | this contract's version, `1` |
| `publicUrl` | string | `PHOTOVAULT_PUBLIC_URL`, no trailing slash |
| `homeUrl` | string \| null | `PHOTOVAULT_HOME_URL` (LAN address) |
| `albumsUrl`, `castUrl` | string | base that public share links use (`PHOTOVAULT_ALBUMS_URL`, else `publicUrl/albums`); cast = `publicUrl/cast` |
| `castAppId` | string \| null | `PHOTOVAULT_CAST_APP_ID`: the server's own Google Cast receiver (registered to `castUrl`). Apps offer Chromecast auto-pairing only when set; they never launch Ente's receiver. null: pair with a code |
| `modelsUrl` | string \| null | base for ML model files, replaces `https://models.ente.com` (see [asset-mirror.md](asset-mirror.md)); null when the mirror is off |
| `musicManifestUrl` | string \| null | replaces `https://music.ente.com/tracks.json`; null when the mirror is off |
| `signupsAllowed` | bool | apps hide "Create account" when false |
| `emailVerification` | `"smtp"` \| `"log"` \| `"skip"` | how codes are delivered |
| `storage` | `"local"` \| `"s3"` | |
| `features` | string[] | unknown entries must be ignored by clients |

Required (already implemented on the server branch): `product`, `name`, `version`, `publicUrl`, `homeUrl`, `signupsAllowed`.
The rest are additive; clients must treat them as optional and ignore unknown fields. Address switching rule (fork and native): if `homeUrl`
answers `/photovault/info` within 1.5 s with the same `publicUrl`, use it; otherwise use `publicUrl`.
Re-check on network change.

QR code payload (web Settings page and app scanner): the URL
`photovault://connect?url=<urlencoded publicUrl>&home=<urlencoded homeUrl>&name=<urlencoded name>`.
Scanners must also accept a bare `http(s)://` URL.

mDNS / DNS-SD: service `_photovault._tcp`, port = the listening port, TXT keys
`name`, `version`, `public` (publicUrl), `home` (homeUrl, optional), `info=/photovault/info`.
Disabled with `PHOTOVAULT_MDNS=false`.
