# Server configuration contract

Precedence: environment variable > `--config` TOML > `/config/keys.toml` (keys only) > default.
`PHOTOVAULT_PUBLIC_URL` is the only required setting.

## Environment variables

| Variable | Default | Meaning |
| --- | --- | --- |
| `PHOTOVAULT_PUBLIC_URL` | **required** | Public base URL of API, web app, albums and cast (one origin) |
| `PHOTOVAULT_HOME_URL` | unset | LAN address, advertised in `/photovault/info` and mDNS |
| `PHOTOVAULT_SERVER_NAME` | `Photovault` | Display name in apps, emails, mDNS |
| `PHOTOVAULT_LISTEN` | `0.0.0.0:8080` | Bind address |
| `PHOTOVAULT_CONFIG_DIR` | `/config` | `keys.toml` and the SQLite database (`photovault.db`) |
| `PHOTOVAULT_CONFIG` | unset | Optional TOML file with the same keys |
| `PHOTOVAULT_DATABASE_URL` | `sqlite:///config/photovault.db` | `postgres://…` to use Postgres |
| `PHOTOVAULT_STORAGE_KIND` | `local` | `local` or `s3` |
| `PHOTOVAULT_STORAGE_PATH` | `/data` | Local object root |
| `PHOTOVAULT_STORAGE_PUBLIC_URL` | = `PHOTOVAULT_PUBLIC_URL` | Where presigned object URLs point |
| `PHOTOVAULT_S3_*` | – | `ENDPOINT`, `PUBLIC_ENDPOINT`, `REGION`, `BUCKET`, `ACCESS_KEY`, `SECRET_KEY`, `SECRET_KEY_FILE`, `PATH_STYLE` (unchanged) |
| `PHOTOVAULT_STORAGE_LIMIT` | unset | S3 total limit (size string, e.g. `4TB`); unset = unlimited |
| `PHOTOVAULT_DEFAULT_QUOTA` | unset | Per-user default quota (size string); unset = none |
| `PHOTOVAULT_WEB_DIR` | `/app/web` (image) | Built web app served on the API port |
| `PHOTOVAULT_WEB_ORIGINS` | unset | Extra CORS origins, comma-separated |
| `PHOTOVAULT_MDNS` | `true` | Announce `_photovault._tcp` |
| `PHOTOVAULT_LOG_FORMAT` | `pretty` | `pretty` or `json`; one line per event |
| `PHOTOVAULT_ALLOW_SIGNUPS` | `false` | Open sign-up |
| `PHOTOVAULT_ADMINS` | unset | Admin user ids (comma-separated) |
| `PHOTOVAULT_SKIP_EMAIL_VERIFICATION` | `false` | Accept any code / skip OTT for single-user installs |
| `PHOTOVAULT_SMTP_HOST`, `_PORT`, `_USERNAME`, `_PASSWORD`, `_PASSWORD_FILE`, `_FROM`, `_TLS` | unset; `_TLS`=`starttls` | `_TLS`: `starttls` \| `tls` \| `none`. Without SMTP the code is logged as `Verification code for <email>: <code>` |
| `PHOTOVAULT_JWT_SECRET`, `PHOTOVAULT_ENCRYPTION_KEY`, `PHOTOVAULT_HASHING_KEY` | from `keys.toml` | If set, used as-is and `keys.toml` is not written |
| `PHOTOVAULT_WEBAUTHN_RP_ID`, `_RP_ORIGINS` | derived from public URL | Passkeys |

Unchanged legacy URL variables (`PHOTOVAULT_ALBUMS_URL`, `_CAST_URL`, `_ACCOUNTS_URL`, `_FAMILY_URL`, …) default to paths under the public URL.

## `/config/keys.toml` (mode 0600, created on first start if missing)

```toml
# Photovault server keys. Back this file up: without it no one can sign in and encrypted
# server-side fields cannot be read.
jwt_secret = "<base64, 32 bytes>"
encryption_key = "<base64, 32 bytes>"   # libsodium secretbox key (museum key.encryption)
hashing_key = "<base64, 64 bytes>"      # museum key.hash
```

Encodings match museum's so an existing install's env values can be pasted in. On creation the server
logs once: `Generated new keys in /config/keys.toml. Back up this file; it cannot be recreated.`

## Startup checks (exit code 78, one line, no restart loop)

Run before serving, in order: config dir writable, storage path writable (local) or bucket reachable (S3),
database reachable and migrated, keys valid (lengths, base64). Message format:

```
photovault: cannot write /data: owned by uid 0, running as uid 65532; fix: chown -R 65532:65532 <host path for /data>
```

Exit with code 78 (EX_CONFIG). The image's compose docs pair this with `restart: unless-stopped`; the
server sleeps 30 s before exiting on a failed check so a restart policy does not spin.

## Admin CLI (`photovault admin <verb>`)

Existing: `list-users`, `set-storage`, `pending-ott`, `add-ott`, `disable-2fa`, `delete-user`, `mark-admin`.
Added: `create-user <email> [--password-stdin]` (runs SRP setup and key generation server-side
exactly as the web client does, see note), `reset-password <email>` (issues a one-time code and prints it;
the user picks the password in the app, because the server never sees passwords or keys),
`quota <email> <size|none>` (alias of `set-storage`), `signups on|off`.
Other: `photovault migrate-to-sqlite --from postgres://… [--to /config/photovault.db]`.

Note: E2EE means the server cannot set a password itself. `create-user` creates the account row and
prints a sign-up code; the user finishes in any app ("Create account" with that code works even when
signups are off). `reset-password` likewise relies on the user's recovery key in the app.
