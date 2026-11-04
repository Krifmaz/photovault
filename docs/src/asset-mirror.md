# Models and music

Face grouping, magic search, pets, text recognition and document scan run on
the phone or desktop with ML models. Memories can play background music. Ente's
apps download both from Ente's CDN; Photovault's apps get them from your server
instead, so nothing contacts Ente.

The server downloads each file the first time an app asks for it, checks it
against the size and SHA-256 the apps pin, and keeps it in `/config/assets`.
After that it never goes back upstream for that file. All models together are
about 330 MB; the server needs internet access for the first download only.

| What | Server address |
| --- | --- |
| Models | `<public URL>/photovault/models/<file>` |
| Music list | `<public URL>/photovault/music/tracks.json` |
| Music tracks | `<public URL>/photovault/music/<file>.mp3` |

Only the files the apps actually use are served; see
[`contracts/asset-mirror.md`](https://github.com/Krifmaz/photovault/blob/main/contracts/asset-mirror.md)
for the list and the HTTP details (Range, ETag).

## Offline servers

Copy the model files into `/config/assets/models/` (same names as on
`models.ente.com`, `PP-OCRv5/ppocrv5_dict.txt` in a subfolder) and the server
serves them without going online. Or point `PHOTOVAULT_MODELS_UPSTREAM` and
`PHOTOVAULT_MUSIC_UPSTREAM` at your own mirror.

The apps read `modelsUrl` and `musicManifestUrl` from `/photovault/info` and
fetch through the address they use at that moment (the home address on your
LAN). Music tracks may be plain `http` when they come from your server.

To turn the feature off: `PHOTOVAULT_ASSET_MIRROR=false`. The apps never fall
back to Ente's hosts: on-device machine learning (faces, magic search) is then
unavailable in the Android app, and memories play without music.
