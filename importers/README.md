# photovault-import

Import an **Immich** library or an **iCloud Photos export** into Photovault, or
into any server that speaks Ente's API. Based on Ente (ente.com).

Everything is encrypted on the machine running the importer, with Ente's own
scheme (libsodium, SRP login, per-file and per-album keys, encrypted metadata),
before anything is uploaded. The server only ever receives ciphertext.

```sh
export PHOTOVAULT_ENDPOINT=https://photos.example.com
export PHOTOVAULT_EMAIL=me@example.com
# Password is prompted for, or read from PHOTOVAULT_PASSWORD.

# Immich (API key from Account Settings → API Keys)
IMMICH_API_KEY=... photovault-import immich --url https://immich.example.com

# iCloud (zips from privacy.apple.com; no need to unzip)
photovault-import icloud ~/Downloads/icloud-export/
```

Re-running is safe: a journal (`photovault-import.jsonl`) skips finished items
before they are downloaded, and content hashes catch anything already in the
library. `--dry-run` prepares and encrypts everything without uploading.

Full guide: [`docs/src/importers.md`](../docs/src/importers.md).

## Layout

| Module | What it does |
| --- | --- |
| `crypto`, `srp` | libsodium wrappers and the SRP-6a client, checked against vectors from Ente's own Go and web code (`tests/vectors/`) |
| `ente` | Ente client: login (SRP, TOTP, email code), albums, favourites, hidden, archive, encrypted (multipart) upload, de-duplication |
| `media` | Type detection, Ente content hash, EXIF/ffprobe metadata, thumbnails, Live Photo bundles |
| `immich` | Immich source (API v1.133 to v3.x) |
| `icloud` | iCloud Photos export source (zips or folders) |
| `state`, `run` | Resume journal and the import pipeline |

## Tests

`cargo test -p photovault-import` runs everything offline: crypto vectors,
mock Immich and mock Ente servers, synthetic iCloud exports, and a whole
pipeline run. Two extra tests need Go and run Ente's own code against ours:

```sh
GO=/usr/local/go/bin/go cargo test -p photovault-import --test crypto_vectors -- --include-ignored
```
