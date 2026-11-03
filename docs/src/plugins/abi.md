# Host/guest ABI (version 1)

This is what a plugin in any language must implement. Rust authors get all of
it from the guest SDK; this page is for other languages with an
[Extism PDK](https://extism.org/docs/concepts/pdk) (Go, AssemblyScript, C,
Zig, JavaScript, ...). The authoritative definitions are the types in
`plugins/sdk/manifest/src/abi.rs`.

## Exports

Each hook declared in the manifest must be exported. Input and output are
JSON.

| Export | Input `event` |
| --- | --- |
| `on_upload` | `{"type": "upload", "file_id": 42, "file": {...} \| null}` (`file` only with `read:metadata`) |
| `on_schedule` | `{"type": "schedule", "last_run": 1730000000000000 \| null}` |
| `on_demand` | `{"type": "on_demand", "action": "apply_all"}` |

Full input:

```json
{
  "event": { "type": "on_demand", "action": "apply_all" },
  "settings": { "rules": "Videos: type=video" },
  "now": 1730000000000000,
  "runner": "desktop"
}
```

Times are microseconds since the Unix epoch (Ente's convention). `runner` is
one of `mobile`, `desktop`, `web`, `node`, `test`.

Output:

```json
{ "summary": "Added 12 photos to 3 albums", "changed": 12 }
```

Return an Extism error to fail the run; its message is shown in the plugin's
activity log.

## Imports (`extism:host/user`)

### `photovault_call(json) -> json`

Request: `{"method": "<name>", "params": {...}}`. Response: `{"ok": <value>}`
or `{"err": {"code": "<code>", "message": "..."}}`.

| Method | Params | Returns | Needs |
| --- | --- | --- | --- |
| `list_files` | `album_id?`, `created_after?`, `created_before?`, `file_type?`, `limit?` (≤500), `cursor?` | `{files, next_cursor}` | `read:metadata` |
| `get_file` | `file_id` | file or `null` | `read:metadata` |
| `list_albums` | – | `[album]` | `read:metadata` |
| `add_tags` | `file_id`, `tags` | `null` | `write:tags` |
| `ensure_album` | `name` | album (existing owned album with that name, or new) | `write:albums` |
| `add_to_album` / `remove_from_album` | `album_id`, `file_ids` | `null` | `write:albums` |
| `trash_files` | `file_ids` | `null` | `write:trash` |
| `export_file` | `file_id`, `path`, `overwrite?` | `{written, bytes}` | `write:export` |
| `kv_get` | `key` | value or `null` | – |
| `kv_set` | `key`, `value` (`null` deletes) | `null` | – |
| `progress` | `done`, `total?`, `message?` | `null` | – |

Error codes: `permission_denied`, `not_found`, `invalid_request`,
`limit_exceeded`, `internal`.

### `photovault_read(json) -> bytes`

Request: `{"file_id": 42, "kind": "thumbnail" | "original"}`. Returns the raw
decrypted bytes. Needs `read:thumbnails` or `read:originals`. On failure the
call raises an Extism error whose message is the JSON error object above.

### HTTP

Use Extism's built-in `http_request`. The host only allows the granted
`network` hosts.
