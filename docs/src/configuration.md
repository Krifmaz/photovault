# Configuration

Photovault reads an optional TOML file (`--config /path/photovault.toml` or
`PHOTOVAULT_CONFIG`), then applies `PHOTOVAULT_*` environment variables on
top. Everything has a default, so a container can run on environment
variables alone. A commented example is in
[`deploy/photovault.example.toml`](https://github.com/Krifmaz/photovault/blob/main/deploy/photovault.example.toml).

> Sections added by other parts of the server (mail templates, push, limits…)
> are documented next to the feature; this page lists the core keys.

## Top level

| Key | Env | Default | Meaning |
| --- | --- | --- | --- |
| `listen` | `PHOTOVAULT_LISTEN` | `0.0.0.0:8080` | Address the HTTP API binds to. |
| `public_url` | `PHOTOVAULT_PUBLIC_URL` | `http://localhost:8080` | API URL as the apps see it, e.g. `https://photos.example.com`. Enter this as the server endpoint in the apps. |
| `database_url` | `PHOTOVAULT_DATABASE_URL` | `sqlite://photovault.sqlite?mode=rwc` | `postgres://user:pass@host/db` or `sqlite:///absolute/path.sqlite?mode=rwc`. |
| `default_storage_bytes` | – | 100 TiB | Quota for new accounts (museum's self-hosted default). |
| `admins` | `PHOTOVAULT_ADMINS` (comma separated) | `[]` | User IDs allowed to call `/admin/*`. |
| `allow_signups` | `PHOTOVAULT_ALLOW_SIGNUPS` (`true`/`1`) | `true` | When false, only the admin CLI creates accounts. |
| `hardcoded_ott` | – | `{}` | Fixed one-time codes per email (`"me@example.com" = "123456"`), like museum's `internal.hardcoded-ott`. For testing. |
| `log_format` | `PHOTOVAULT_LOG_FORMAT` | `json` | `json` (one object per line) or `pretty`. Filter with `RUST_LOG` (default `info`). |

## `[secrets]`

All values are standard base64. **Set them in production.** If one is empty a
random value is generated at start-up, which is fine for a test run but means
every restart logs everyone out and — for `encryption`/`hashing` — makes stored
email addresses unreadable.

| Key | Env | Size | Used for |
| --- | --- | --- | --- |
| `jwt` | `PHOTOVAULT_JWT_SECRET` | 32 bytes | HMAC key for JWTs and for signing local-storage URLs. |
| `encryption` | `PHOTOVAULT_ENCRYPTION_KEY` | 32 bytes | libsodium secretbox key for email addresses at rest. |
| `hashing` | `PHOTOVAULT_HASHING_KEY` | 32+ bytes | libsodium generichash key for email lookup hashes. |

Generate each with `head -c 32 /dev/urandom | base64`.

## `[storage]`

`kind` selects the backend. See [Storage backends](storage.md) for behaviour.

### `kind = "local"` (default)

| Key | Env | Default | Meaning |
| --- | --- | --- | --- |
| `path` | `PHOTOVAULT_STORAGE_PATH` | `data` | Directory for objects (a mergerfs pool is fine). |
| `public_url` | `PHOTOVAULT_STORAGE_PUBLIC_URL` | `public_url` | Base URL put into signed upload/download URLs, e.g. `https://storage.example.com`. |

### `kind = "s3"`

| Key | Env | Default | Meaning |
| --- | --- | --- | --- |
| `endpoint` | `PHOTOVAULT_S3_ENDPOINT` | – (required) | URL the **server** uses, e.g. `https://s3.eu-central-003.backblazeb2.com` or `http://garage:3900`. |
| `public_endpoint` | `PHOTOVAULT_S3_PUBLIC_ENDPOINT` | `endpoint` | URL put into presigned URLs for the **apps**, when it differs (internal Docker name vs public hostname). |
| `region` | `PHOTOVAULT_S3_REGION` | `us-east-1` | Signing region (`garage` for Garage, the B2 region for B2, …). |
| `bucket` | `PHOTOVAULT_S3_BUCKET` | – (required) | Bucket name. |
| `access_key` | `PHOTOVAULT_S3_ACCESS_KEY` | – (required) | Access key id. |
| `secret_key` | `PHOTOVAULT_S3_SECRET_KEY` | – | Secret access key. |
| `secret_key_file` | `PHOTOVAULT_S3_SECRET_KEY_FILE` | – | Read the secret from a file instead (Docker secrets). |
| `path_style` | `PHOTOVAULT_S3_PATH_STYLE` | `true` | `endpoint/bucket/key` URLs; set `false` for `bucket.endpoint/key`. |

Setting `PHOTOVAULT_STORAGE_KIND=s3` (or `local`) switches backend from the
environment; the other variables then patch that backend's settings.

## `[smtp]`

Without SMTP, emails (including login codes) are written to the log.

| Key | Meaning |
| --- | --- |
| `host`, `port` | Relay (465 implicit TLS, 587 STARTTLS). |
| `username`, `password` | Optional credentials. |
| `from` | Sender address. |

## Other environment variables

| Env | Meaning |
| --- | --- |
| `PHOTOVAULT_CONFIG` | Path of the TOML file. |
| `PHOTOVAULT_HEALTHCHECK_URL` | URL probed by `photovault healthcheck` (default `http://127.0.0.1:8080/healthz`). |
| `RUST_LOG` | Log filter, e.g. `info,sqlx=warn`. |
| `SSL_CERT_FILE` | CA bundle used instead of the system store for HTTPS to an S3 endpoint (e.g. a private CA). |
