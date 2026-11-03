# Architecture

```text
  Ente apps (mobile / web / desktop, unmodified)
     │  JSON API (museum-compatible)          │  encrypted bytes via signed URLs
     ▼                                        ▼
  ┌──────────────────────────────┐     ┌───────────────────────────────┐
  │ photovault (axum)            │     │ object store                  │
  │  routes/<group>.rs           │────▶│  local dir  → /storage/* here │
  │  auth · error · crypto · db  │     │  S3 bucket  → presigned SigV4 │
  └──────────────┬───────────────┘     └───────────────────────────────┘
                 ▼
        SQLite or PostgreSQL (sqlx AnyPool)
```

## Principles

* **Ciphertext only.** The apps encrypt with libsodium before upload. The
  server stores opaque blobs and encrypted metadata, and never needs a key
  that could decrypt user content.
* **Museum compatibility.** Request/response JSON, status codes and error
  `code` strings follow Ente's Go server so the stock apps work unchanged.
  When museum and the apps disagree, the apps win.
* **Small footprint.** One static-ish binary, distroless image, SQLite or
  Postgres, local disk or S3.

## Server layout (`server/src`)

| Module | Role |
| --- | --- |
| `main.rs` | CLI: `serve`, `migrate`, `healthcheck`, `admin`. Graceful shutdown on SIGTERM; starts storage housekeeping. |
| `lib.rs` | `app()`: router plus global middleware (CORS, request ids, access log + metrics). |
| `config.rs` | TOML + `PHOTOVAULT_*` environment configuration. |
| `state.rs` | `AppState`: database, object store, secrets, metrics. |
| `routes/<group>.rs` | One module per museum endpoint group, each exposing `router()`. |
| `auth.rs` | Token extractors (`AuthUser`, `StorageUser`, `AdminUser`). |
| `error.rs` | `ApiError` → `{"code","message"}` like museum. |
| `db.rs` | `sqlx::AnyPool` over Postgres and SQLite; portability rules; migrations in `server/migrations/{postgres,sqlite}`. |
| `crypto.rs` | libsodium wrappers (random, sealed boxes, secretbox, generichash). |
| `storage/` | `ObjectStore` trait, `local` and `s3` backends, `metered` wrapper. |
| `metrics.rs` | Prometheus registry. |

## Uploads and downloads

1. The app asks the API for upload URLs (`/files/upload-urls`,
   `/files/upload-url` with `contentLength`+`contentMD5`, or the multipart
   variants). The API creates object keys `<userID>/<uuid>` and returns signed
   URLs from `ObjectStore`.
2. The app `PUT`s ciphertext to those URLs, sending `Content-MD5` when it was
   bound. For multipart uploads it collects each part's `ETag` and `POST`s a
   `CompleteMultipartUpload` XML document to the completion URL.
3. The app registers the file (`POST /files`) with the object keys and sizes;
   the server checks the objects exist and charges the quota.
4. Downloads use `GET /files/download/{id}` and friends, which redirect to a
   signed GET URL.

The **local** backend serves those URLs itself under `/storage/*`
(`routes/objects.rs`), emulating the S3 behaviour the apps depend on. The
**S3** backend presigns real S3 URLs with its own SigV4 implementation; TLS
for server-side S3 calls lives only in `storage/s3/http.rs`.

## Data model

Tables mirror museum's names and columns (`users`, `tokens`, `files`,
`collections`, …) so behaviour can be compared side by side, simplified
where museum carries cloud-only features (billing, multiple data centres).
Times are microseconds since the epoch; binary values are base64 text.

## Repository layout

| Path | Contents |
| --- | --- |
| `server/` | The Rust server |
| `mobile/`, `web/`, `desktop/` | Light forks of Ente's apps, kept in sync with [the upstream merge](upstream.md) |
| `deploy/` | Compose files and example configuration |
| `docs/` | This book (`mdbook build docs`) |
| `scripts/` | Upstream merge, S3 test harness |
