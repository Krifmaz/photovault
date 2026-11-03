# Photovault shared contracts (v1)

Every track (server, fork-mobile, fork-web, core, native-android, perf) builds against
these files. They are the only cross-track interfaces. Change them only in a
`contracts:` commit merged into the default branch first; tracks then rebase.

| File | What it fixes |
| --- | --- |
| [`server-info.md`](server-info.md), [`server-info.schema.json`](server-info.schema.json) | `GET /photovault/info` (public) |
| [`storage-usage.md`](storage-usage.md), [`storage-usage.schema.json`](storage-usage.schema.json) | `GET /photovault/usage` and the real numbers in Ente's `/users/details/v2`, `/billing/subscription` |
| [`server-env.md`](server-env.md) | Server environment variables, `/config/keys.toml`, admin CLI verbs, startup-check messages |
| [`../branding/encore-tokens.json`](../branding/encore-tokens.json) | Encore colours, fonts, type scale, radii, spacing, motion (shared with the fork's existing file) |
| [`photovault-core.md`](photovault-core.md) | photovault-core UniFFI (proc-macro) interface consumed by the native app |
| [`apps.json`](apps.json) | App names, Android package IDs, build-time defines, mDNS service type |

Rules that apply to every track:

* All apps use Ente's encryption exactly (libsodium, SRP-6a 4096, per-file and per-collection keys,
  encrypted metadata). Nothing in these contracts changes a byte on the wire for Ente's own API paths.
* The only new server paths are under `/photovault/`. Ente's API paths are unchanged.
* JSON field names are camelCase, sizes are bytes as integers, times are Unix microseconds
  (Ente's convention) unless named `...Seconds`.

## Branches and merge order

Default branch: `claude/project-thread-m3y3rf`. Each track merges the default branch into its own
branch after this lands, opens a PR into the default branch, and runs its own tests first.
Merge order: contracts → server, core → fork-mobile, fork-web, native-android → perf → verify.
Android builds run in GitHub Actions (the dev containers cannot reach Google's Maven).
