# Screenshot cleanup (`dev.photovault.screenshot-cleanup`)

Example Photovault plugin that finds old screenshots and collects them in an
album or moves them to the trash.

**Permissions:** read metadata, write albums, move to trash. Trash is marked
as sensitive on the install screen; trashed files stay recoverable from the
trash, as in Ente.

## Settings

| Setting                     | Default       | Meaning                                                    |
|-----------------------------|---------------|------------------------------------------------------------|
| What to do with screenshots | album         | `album`: add them to an album; `trash`: move to the trash  |
| Album name                  | `Screenshots` | album to collect into (created if missing)                 |
| Only screenshots older than | 30 days       | age by capture time                                        |
| Never touch favourites      | on            | favourites are never added or trashed                      |

## Hooks

* **Clean up screenshots now** (asks for confirmation first).
* A daily schedule.

## What counts as a screenshot

Only still images (screen recordings are left alone) where either a segment of
the device folder, compared case-insensitively, is one of

`Screenshots`, `Screenshot`, `Screen Shots`, `Capturas de pantalla`, `Bildschirmfotos`

or the file name starts with one of these, followed by a non-letter:

| Prefix                  | Produced by                                                     |
|-------------------------|-----------------------------------------------------------------|
| `Screenshot`            | Android `Screenshot_…`, macOS 10.14+, Windows 11, GNOME          |
| `Screen Shot`           | macOS before 10.14                                              |
| `Simulator Screen Shot` | Xcode's iOS simulator                                           |
| `Captura de pantalla`   | Spanish macOS and Windows                                       |
| `Bildschirmfoto`        | German macOS                                                    |

On iOS, screenshots are found through the "Screenshots" album they were backed
up from. The lists are short on purpose; add your own language in
`SCREENSHOT_FOLDERS` / `SCREENSHOT_PREFIXES` and send a patch.

## Build and test

```sh
cd plugins/sdk/guest
cargo build --release -p photovault-plugin-screenshot-cleanup
cargo test -p photovault-plugin-screenshot-cleanup --target x86_64-unknown-linux-gnu
```

The run collects matching ids first and acts after walking the library, so
trashing never shifts the pages being read.
