# Model and music mirror

The apps download on-device ML models from `models.ente.com` and memories
music from `music.ente.com`. A Photovault server serves the same files itself,
so the apps never contact Ente. Advertised in `/photovault/info` as
`features: ["models", "music"]` plus `modelsUrl` and `musicManifestUrl`.
`PHOTOVAULT_ASSET_MIRROR=false` turns it off (routes 404, fields null).

All routes are public (no auth; the files are public upstream and contain no
user data), `GET` and `HEAD`.

## `GET /photovault/models/<file>`

Same `<file>` as `https://models.ente.com/<file>`. Allowed files (the apps' own
catalog in `rust/crates/ml`, with pinned size and SHA-256):

| File | Used for |
| --- | --- |
| `yolov5s_face_640_640_static_b1.onnx`, `mobilefacenet_portable_static_b1.onnx` | faces |
| `mobileclip_s2_image_gelu_opset20.onnx`, `mobileclip_s2_text_opset18_quant.onnx`, `bpe_simple_vocab_16e6.txt` | magic search |
| `yolov5s_pet_face_fp16_V2.onnx`, `dog_face_embedding128.onnx`, `cat_face_embedding128.onnx`, `yolov5s_object_fp16.onnx`, `dog_body_embedding192.onnx`, `cat_body_embedding192.onnx` | pets |
| `det_fixed_v1.onnx`, `cls_fixed_v1.onnx`, `rec_fixed_v1.onnx`, `PP-OCRv5/ppocrv5_dict.txt` | text recognition |
| `document_segmentation_opt.onnx` | document scan |

Anything else: 404. The first request downloads from upstream (the client
waits), checks size and SHA-256, and stores it in `<config_dir>/assets/models/`;
a mismatch or unreachable upstream gives 502 and nothing is stored.

## `GET /photovault/music/tracks.json`

Ente's manifest (`{"tracks": [{"id", "url", "title"?, "artist"?}, ...]}`),
refreshed from upstream at most hourly (the last good copy is used while
upstream is down). Every track hosted on `music.ente.com` gets
`url = <publicUrl>/photovault/music/<path>`; tracks hosted anywhere else are
removed. Note the stock app only accepts `https` track URLs.

## `GET /photovault/music/<path>.mp3`

Only paths listed in the current manifest. Cached in
`<config_dir>/assets/music/tracks/` (max 64 MiB per track).

## Caching headers (models and tracks)

* `ETag: "<sha256 hex>"` (strong); `If-None-Match` gives 304.
* `Range: bytes=...` gives 206; `If-Range` with the ETag is honoured
  (a different ETag returns the whole file).
* `Cache-Control: public, max-age=31536000, immutable`; `Accept-Ranges: bytes`.

## Upstreams

`PHOTOVAULT_MODELS_UPSTREAM` (default `https://models.ente.com`) and
`PHOTOVAULT_MUSIC_UPSTREAM` (default `https://music.ente.com`), e.g. for an
offline mirror. Pre-seeding works too: copy the files into
`<config_dir>/assets/models/` (they are served as is, ETag = pinned hash).
