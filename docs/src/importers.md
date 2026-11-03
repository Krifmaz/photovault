# Importers

`photovault-import` brings an Immich library or an iCloud Photos export into
your Photovault account. It logs in like an Ente app, encrypts every file,
thumbnail and piece of metadata on your computer, and then uploads only
ciphertext. It works with any Ente-compatible server.

## Common options

| Option | Env | Default | Meaning |
| --- | --- | --- | --- |
| `--endpoint` | `PHOTOVAULT_ENDPOINT` | | API address, e.g. `https://photos.example.com` |
| `--email` | `PHOTOVAULT_EMAIL` | prompt | Account email |
| `--password-env` | | `PHOTOVAULT_PASSWORD` | Variable to read the password from; prompted if unset |
| `--journal` | | `photovault-import.jsonl` | Resume journal |
| `--default-album` | | "Imported from Immich/iCloud" | Album for items in no album |
| `--people` | | `albums` | `albums` (one album per person), `caption` (append "People: …"), `skip` |
| `--people-prefix` | | empty | Prefix for person albums, e.g. `"Person: "` |
| `--dry-run` | | off | Encrypt and prepare, upload nothing |
| `--json-logs` | | off | Structured logs |

Two-factor (TOTP) and email-code logins are prompted for. Passkey-only
accounts are not supported by the importer.

**How things map to Ente**

- Albums become Ente albums (found by name, or created).
- Favourites go into your Favourites collection.
- Archived items are archived in Ente.
- Hidden items go only into Ente's hidden album.
- Descriptions become the caption.
- Dates, GPS and dimensions go into the encrypted metadata.
- Live Photos become Ente Live Photos.

**Safe to re-run.** Items listed in the journal are skipped before they are
downloaded. Items whose content hash is already in your library are not
uploaded again; they are only added to any missing albums. Items that fail are
logged, and the importer exits with code 2 so you can run it again.

**External tools (optional).** `ffmpeg` and `ffprobe` provide video thumbnails,
dates and locations. `heif-convert`, ImageMagick or ffmpeg 7 or later provide
HEIC thumbnails. Without them, a placeholder thumbnail is used; the original is
still imported unchanged.

## Immich

```sh
IMMICH_API_KEY=... photovault-import immich --url https://immich.example.com
```

Create the key in Immich under **Account Settings → API Keys**. It needs these
permissions:

- `user.read`
- `album.read`
- `asset.read`
- `asset.view`
- `asset.download`
- `asset.statistics` (optional, for the progress total)

Works with Immich v1.133 and later, including v3.x.

**What is imported**

- Originals.
- Live Photos. The motion video is not imported a second time, and Android
  motion photos stay as the photo.
- Albums, including those shared with you.
- People names: named, non-hidden people only. Faces are re-detected on your
  devices.
- Favourites, archive state and descriptions.
- GPS, capture time and dimensions.

**Not imported**

- Trash.
- Partners' and other members' photos.
- Edits.
- Tags, ratings, stacks, face positions, memories, shared links and activity.
- The Locked Folder. `--include-locked` tries to import it into the hidden
  album, but Immich normally hides it from API keys.

| Flag | Meaning |
| --- | --- |
| `--include-locked` | Also try the Locked Folder (into the hidden album) |
| `--no-immich-thumbnails` | Make thumbnails locally instead of using Immich's previews |
| `--page-size` | Assets per request (default 250) |
| `--naive-times local` | Read times without a time zone in this computer's zone, as Ente does (default: Immich's interpretation) |
| `--temp-dir` | Where originals are downloaded |

## iCloud Photos

1. Go to [privacy.apple.com](https://privacy.apple.com) and choose
   **Request a copy of your data**.
2. Tick **iCloud Photos** and choose the largest part size.
3. When Apple's email arrives, download every `iCloud Photos Part N of M.zip`
   into one folder. You don't need to unzip them.
4. Run:

```sh
photovault-import icloud ~/Downloads/icloud-export/
```

You can pass zips, extracted part folders, or a folder that holds them.

**What is imported**

- Originals.
- Live Photos: the still and the video are rejoined, even when they are in
  different parts.
- Albums, including shared albums, imported as albums of the same name.
- Favourites.
- Hidden photos, into the hidden album.
- Capture dates: Apple's date, refined from EXIF.
- GPS from photos and videos.
- Photo descriptions.

Items in Recently Deleted are skipped.

**Limits**

- Apple's export has no people, keywords or edits; only originals are
  imported.
- Shared-album copies are lower quality and have no location.
- Files Apple renamed `-1` because of duplicate names are matched by order.
- An incomplete part zip is reported and must be downloaded again.
- Apple's CSV dates say "GMT" and are read as UTC. EXIF is preferred when the
  two agree within a minute.

| Flag | Meaning |
| --- | --- |
| `--no-live-photos` | Import stills and videos separately |
| `--no-shared-albums` | Skip shared albums |
| `--memories-as-albums` | Each memory becomes an album "Memory: name" |
| `--include-edited` | Also import `IMG_E1234.*` edited copies |
| `--temp-dir` | Where zip entries are extracted |

## Compatibility notes for server implementers

- Uploads use `POST /files/upload-url` and `POST /files/multipart-upload-url`
  with `Content-MD5`, as current Ente apps do. The importer falls back to the
  older `GET /files/upload-urls` endpoints on 404 or 405.
- Files of 5 or more chunks (over 16 MiB) use multipart.
- Archive state is set afterwards with `PUT /files/magic-metadata`.
- The hidden album is an `album` named `.hidden`, with magic metadata
  `{subType: 1, visibility: 2}`.
