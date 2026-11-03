# Auto albums (`dev.photovault.auto-albums`)

Example Photovault plugin that sorts files into albums by rules you write.

**Permissions:** read metadata, write albums. Nothing else: it cannot see
photos, trash anything or reach the network.

## Rules

One rule per line in the **Rules** setting:

```text
# Lines starting with # are comments.
Screenshots: folder=Screenshots
Dogs: name=*dog* and type=image
Summer in Rome: near=41.9,12.5,30 and month=6-8
Old favourites: favorite and year=2000-2015
Clips: type=video|live_photo and not folder=WhatsApp*
```

`Album name: condition and condition ...`. A file goes into the album when
every condition holds; several lines with the same album name are
alternatives. Keywords are case-insensitive; album names cannot contain `:`.

| Condition               | Holds when                                                   |
|-------------------------|--------------------------------------------------------------|
| `type=image`            | file type is `image`, `video` or `live_photo`               |
| `folder=<glob>`         | the device folder it was backed up from matches             |
| `name=<glob>`           | the file name matches (case-insensitive)                     |
| `year=2024`             | capture year, UTC (`2019-2021` ranges allowed)               |
| `month=6`               | capture month 1-12, UTC (`6-8` ranges allowed)               |
| `favorite`              | the file is a favourite                                      |
| `archived`              | the file is archived                                         |
| `near=<lat>,<lon>,<km>` | the file has GPS within `km` kilometres (great-circle)       |
| `not <condition>`       | the condition does not hold                                  |

`key=value` conditions accept alternatives with `|` (`name=*.png|*.gif`).
Globs use `*` and `?` and match the whole name.

Mistakes are reported all at once, with line numbers, in the activity log
(for example `2 invalid rules: line 3: unknown condition `colour`; line 5:
month `13` must be a number from 1 to 12, ...`), and nothing is changed until
they are fixed. Use **Check rules** to see what would happen without changing
anything.

## Hooks

| Hook                       | What it does                                                         |
|----------------------------|----------------------------------------------------------------------|
| upload                     | applies the rules to the new file                                    |
| **Apply rules to the whole library** | walks every file, with progress                            |
| **Check rules**            | the same walk, only reporting what it would add                      |
| daily schedule             | the same as Apply, for files uploaded from other devices             |

Runs are idempotent: a file already in an album is never re-added, and an
album is created only when the first file matches it (it reuses your existing
album with the same name).

## Build and test

```sh
cd plugins/sdk/guest
cargo build --release -p photovault-plugin-auto-albums
cargo test -p photovault-plugin-auto-albums --target x86_64-unknown-linux-gnu
```

The logic lives in plain functions (`rules::parse_rules`, `Condition::matches`,
`apply`) tested natively with the PDK's `MockHost`.
