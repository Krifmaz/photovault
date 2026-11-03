# The plugin manifest

Every plugin ships a `plugin.toml`. It is signed together with the Wasm module,
so it cannot be changed after packing.

```toml
[plugin]
id = "dev.photovault.auto-albums"     # reverse-DNS, stable across versions
name = "Auto albums"
version = "0.1.0"                     # MAJOR.MINOR.PATCH
description = "Sort photos into albums by rules."
authors = ["Photovault contributors"]
license = "AGPL-3.0-only"
api_version = 1                       # must match the host

[permissions]
read = ["metadata"]                   # metadata | thumbnails | originals
write = ["albums"]                    # tags | albums | trash | export
network = []                          # host patterns, e.g. "api.example.com", "*.example.com"

[hooks]
on_upload = true
schedule = { every_minutes = 1440 }   # at least 15
on_demand = [
  { id = "apply_all", label = "Apply rules to whole library", confirm = true },
]

[limits]
memory_mb = 32                        # host clamps to its own maximum
timeout_ms = 20000

[[settings]]
key = "rules"
label = "Rules"
type = "text"
multiline = true
help = "One rule per line: Album name: condition and condition"
```

## `[plugin]`

| Key | Required | Notes |
| --- | --- | --- |
| `id` | yes | Two or more labels of `a-z`, `0-9`, `-`, separated by dots. Updates must keep it. |
| `name` | yes | 1–64 characters. |
| `version` | yes | Strict `MAJOR.MINOR.PATCH`; updates must be strictly higher. |
| `api_version` | yes | `1`. |
| `description`, `authors`, `license`, `homepage` | no | Shown in the plugin list. |

## `[hooks]`

| Key | Export the module must provide |
| --- | --- |
| `on_upload = true` | `on_upload` |
| `schedule = { every_minutes = N }` | `on_schedule` |
| `on_demand = [{ id, label, confirm? }]` | `on_demand` (one button per action) |

## `[[settings]]`

The app renders the settings page with its own themed widgets. Field types:

| `type` | Value | Extra keys |
| --- | --- | --- |
| `toggle` | bool | `default` |
| `text` | string | `default`, `multiline`, `max_length` (default 4096), `placeholder` |
| `number` | number | `default`, `min`, `max`, `integer`, `unit` |
| `select` | string | `options = [{ value, label }]`, `default` |
| `album` | album id, or array with `multiple = true` | `multiple` |

Every field also takes `key` (`a-z0-9_`), `label` and optional `help`. Hooks
always receive a complete, valid settings object with defaults filled in.
