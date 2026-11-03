# Export albums (`dev.photovault.export-albums`)

Example Photovault plugin that keeps decrypted copies of the albums you pick in
a folder you choose (a NAS share, an external disk, ...), copying only new and
changed files on each run.

**Permissions:** read metadata, write to the export folder. The plugin never
sees file contents or the real folder path: it asks the host to copy file *N*
to `Album/2024/IMG_0001.HEIC`, and the host decrypts and writes it under the
folder you picked for this plugin. The sandbox has no file system access.

## Settings

| Setting             | Default      | Meaning                                                          |
|---------------------|--------------|------------------------------------------------------------------|
| Albums to export    | none         | album picker, several allowed                                    |
| Folder layout       | album / file | or album / year / file, or album / year-month / file (UTC)       |
| Update changed files| off          | re-copy a file whose modification time changed since its export  |

## Hooks

* **Export now**.
* A daily schedule.

## Paths

* Album names and file names are cleaned before use: `/ \ : * ? " < > |` and
  control characters become `_`, leading dots, trailing dots and spaces are
  dropped, Windows device names (`CON`, `NUL`, ...) get a `_` prefix, and long
  names are shortened keeping the extension. The host checks every path again.
* Two albums or two files that clean to the same name (compared
  case-insensitively, as macOS and Windows do) get ` (2)`, ` (3)`, ... before
  the extension, in album-id and file-id order, so every device produces the
  same result.
* What was exported is remembered in the plugin's on-device storage, so an
  album keeps its folder when renamed, a file keeps its path when the layout
  changes, and reruns only copy what is new.
* A file already present at the destination that this plugin did not record
  (for example your own copy) is never overwritten; it is remembered as
  exported.
* Files removed from an album, or trashed, are not deleted from the export.

## Build and test

```sh
cd plugins/sdk/guest
cargo build --release -p photovault-plugin-export-albums
cargo test -p photovault-plugin-export-albums --target x86_64-unknown-linux-gnu
```
