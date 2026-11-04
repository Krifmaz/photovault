# Configuration

Photovault is configured with environment variables. `PHOTOVAULT_PUBLIC_URL`
is the only one you must set; everything else has a default.

An optional TOML file (`--config /path/photovault.toml` or
`PHOTOVAULT_CONFIG`) takes the same settings. A commented example is in
[`deploy/photovault.example.toml`](https://github.com/Krifmaz/photovault/blob/main/deploy/photovault.example.toml).

## Precedence

For each setting the first of these that is set wins:

1. environment variable (`PHOTOVAULT_*`);
2. the `--config` TOML file;
3. `/config/keys.toml` (only the three keys);
4. the default.

An empty environment variable counts as unset.

## Environment variables

Defaults are those of the container image. Sizes such as `500GB` or `2TB`
use binary units (1 GB = 1024³ bytes); `none`, `unlimited` or `0` mean no
limit. Flags accept `true`, `1`, `yes` or `on`; anything else is false.

### Server

| Variable | Default | Meaning |
| --- | --- | --- |
| `PHOTOVAULT_PUBLIC_URL` | **required** | Address of the API, web app, public albums and cast, e.g. `https://photos.example.com` or `http://10.0.1.7:8095`. This is what you enter in the apps. |
| `PHOTOVAULT_HOME_URL` | unset | LAN address, e.g. `http://10.0.1.7:8095`. Advertised in `/photovault/info` and mDNS; the apps use it when it answers. |
| `PHOTOVAULT_SERVER_NAME` | `Photovault` | Name shown in the apps, in emails and on the LAN. |
| `PHOTOVAULT_LISTEN` | `0.0.0.0:8080` | Address and port the server binds to. |
| `PHOTOVAULT_CONFIG_DIR` | `/config` | Folder for `keys.toml` and the SQLite database `photovault.db`. |
| `PHOTOVAULT_CONFIG` | unset | Path of an optional TOML file. |
| `PHOTOVAULT_DATABASE_URL` | `sqlite:///config/photovault.db?mode=rwc` | Set `postgres://user:password@host:5432/db` to use PostgreSQL. |
| `PHOTOVAULT_WEB_DIR` | `/app/web` | Built web app, served on the API port. Unset or missing folder: API only. |
| `PHOTOVAULT_WEB_ORIGINS` | unset | Origins allowed by CORS, comma-separated. Unset: any origin. `/photovault/info` always allows any origin. |
| `PHOTOVAULT_MDNS` | `true` | Announce `_photovault._tcp` on the LAN. See [LAN discovery](discovery.md). |
| `PHOTOVAULT_ASSET_MIRROR` | `true` | Serve the ML models and memories music to the apps, so they never contact Ente. Each file is downloaded once into `/config/assets` (about 330 MB for all models). See [Models and music](asset-mirror.md). |
| `PHOTOVAULT_MODELS_UPSTREAM` | `https://models.ente.com` | Where the mirror downloads models from. |
| `PHOTOVAULT_MUSIC_UPSTREAM` | `https://music.ente.com` | Where the mirror downloads the memories music from. |
| `PHOTOVAULT_LOG_FORMAT` | `pretty` | `pretty` (plain text) or `json`. One line per event. |
| `RUST_LOG` | `info` | Log filter, e.g. `info,sqlx=warn`. |

### Accounts

| Variable | Default | Meaning |
| --- | --- | --- |
| `PHOTOVAULT_ALLOW_SIGNUPS` | `false` | Let anyone create an account. `photovault admin signups on\|off` overrides it at run time. |
| `PHOTOVAULT_ADMINS` | unset | User ids allowed to call the `/admin/*` API, comma-separated. |
| `PHOTOVAULT_DEFAULT_QUOTA` | unset | Quota for new accounts (size). Unset: no quota, the free space is the limit. |
| `PHOTOVAULT_SKIP_EMAIL_VERIFICATION` | `false` | Accept any code at the email step and send no email. For single-user installs. |

### Storage

| Variable | Default | Meaning |
| --- | --- | --- |
| `PHOTOVAULT_STORAGE_KIND` | `local` | `local` or `s3`. |
| `PHOTOVAULT_STORAGE_PATH` | `/data` | Folder for the encrypted objects (local backend). A mergerfs pool is fine. |
| `PHOTOVAULT_STORAGE_PUBLIC_URL` | `PHOTOVAULT_PUBLIC_URL` | Base of the signed upload/download URLs (local backend), e.g. `https://storage.photos.example.com`. |
| `PHOTOVAULT_STORAGE_LIMIT` | unset | Total the server may store (size). Gives real numbers on S3; also caps a local disk. Unset: free disk space (local) or unlimited (S3). |
| `PHOTOVAULT_S3_ENDPOINT` | – | S3 URL the server uses, e.g. `https://s3.eu-central-003.backblazeb2.com` or `http://garage:3900`. Required for S3. |
| `PHOTOVAULT_S3_PUBLIC_ENDPOINT` | `PHOTOVAULT_S3_ENDPOINT` | S3 URL put into presigned URLs for the apps, when it differs. |
| `PHOTOVAULT_S3_REGION` | `us-east-1` | Signing region (`garage` for Garage). |
| `PHOTOVAULT_S3_BUCKET` | – | Bucket name. Required for S3. |
| `PHOTOVAULT_S3_ACCESS_KEY` | – | Access key id. |
| `PHOTOVAULT_S3_SECRET_KEY` | – | Secret access key. |
| `PHOTOVAULT_S3_SECRET_KEY_FILE` | – | Read the secret key from this file instead (Docker secrets). |
| `PHOTOVAULT_S3_PATH_STYLE` | `true` | `false` for virtual-host URLs (`bucket.endpoint/key`). |

See [Storage backends](storage.md) for how each backend behaves and how the
storage numbers are worked out.

### Email

Setting `PHOTOVAULT_SMTP_HOST` turns email on. Without it nothing is sent and
each sign-in code is written to the log as
`Verification code for <email>: <code>`.

| Variable | Default | Meaning |
| --- | --- | --- |
| `PHOTOVAULT_SMTP_HOST` | unset | SMTP relay host name. |
| `PHOTOVAULT_SMTP_PORT` | `587` | Port. |
| `PHOTOVAULT_SMTP_USERNAME` | unset | Login. |
| `PHOTOVAULT_SMTP_PASSWORD` | unset | Password. |
| `PHOTOVAULT_SMTP_PASSWORD_FILE` | unset | Read the password from this file instead. |
| `PHOTOVAULT_SMTP_FROM` | `Photovault <username>` when the username is an address | Sender, e.g. `Photovault <photos@example.com>`. |
| `PHOTOVAULT_SMTP_TLS` | `tls` on port 465, else `starttls` | `starttls`, `tls` (implicit TLS) or `none` (only for a relay on the same host). |

At start the server tries to connect once and logs a warning if it cannot.

### Keys

| Variable | Default | Meaning |
| --- | --- | --- |
| `PHOTOVAULT_JWT_SECRET` | from `keys.toml` | Signs tokens and storage URLs. |
| `PHOTOVAULT_ENCRYPTION_KEY` | from `keys.toml` | Encrypts email addresses at rest (32 bytes). |
| `PHOTOVAULT_HASHING_KEY` | from `keys.toml` | Hashes email addresses for lookup. |

If all three are set they are used as they are and `keys.toml` is not
touched. `photovault gen-keys` prints three fresh values.

### Companion app URLs and passkeys

These default to museum's local values. You only need them if you host the
accounts, family or cast apps somewhere else, or use passkeys.

| Variable | Default | Meaning |
| --- | --- | --- |
| `PHOTOVAULT_ALBUMS_URL` | `PHOTOVAULT_PUBLIC_URL` + `/shared-albums` in `/photovault/info` | Public albums; share links point here. |
| `PHOTOVAULT_CAST_URL` | `PHOTOVAULT_PUBLIC_URL` + `/cast` in `/photovault/info` | Cast receiver. |
| `PHOTOVAULT_CAST_APP_ID` | unset | Your own Google Cast receiver app id (8 hex digits), for Chromecast auto-pairing. Unset: pair with a code. See [Casting to a TV](chromecast.md). |
| `PHOTOVAULT_ACCOUNTS_URL` | `http://localhost:3001` | Accounts app (passkeys, recovery). |
| `PHOTOVAULT_FAMILY_URL` | `http://localhost:3003` | Family app. |
| `PHOTOVAULT_EMBED_ALBUMS_URL`, `PHOTOVAULT_PASTE_URL`, `PHOTOVAULT_LOCKER_SHARE_URL`, `PHOTOVAULT_PUBLIC_MEMORIES_URL`, `PHOTOVAULT_CUSTOM_DOMAIN_CNAME` | unset | Other companion apps (museum's `apps.*` keys). |
| `PHOTOVAULT_WEBAUTHN_RP_ID` | `localhost` | Passkey relying party id: your domain, e.g. `photos.example.com`. |
| `PHOTOVAULT_WEBAUTHN_RP_ORIGINS` | `http://localhost:3001` | Allowed passkey origins, comma-separated. |

### Commands

| Variable | Default | Meaning |
| --- | --- | --- |
| `PHOTOVAULT_HEALTHCHECK_URL` | `http://127.0.0.1:8080/healthz` | URL probed by `photovault healthcheck` (the image's `HEALTHCHECK`). |
| `PHOTOVAULT_ANNOUNCE_SERVER` | `http://127.0.0.1:8080` | Server read by `photovault announce` (the mDNS sidecar). |
| `PHOTOVAULT_ANNOUNCE_PORT` | port of the home address | Port announced by `photovault announce`. |
| `SSL_CERT_FILE` | system store | CA bundle for HTTPS to an S3 endpoint (e.g. a private CA). |

## `/config/keys.toml`

On first start the server creates this file with mode `0600` and logs once:

```
Generated new keys in /config/keys.toml. Back up this file; it cannot be recreated.
```

```toml
jwt_secret = "<base64, 32 bytes>"
encryption_key = "<base64, 32 bytes>"   # libsodium secretbox key
hashing_key = "<base64, 64 bytes>"      # generichash key
```

Back it up with the database. Without it no one can sign in and stored email
addresses cannot be read. Photos stay safe either way: they are encrypted
with keys that only your devices hold.

The values use museum's encoding, so the `key.encryption`, `key.hash` and
`jwt.secret` of an existing install can be pasted in. Environment variables
override the file, one key at a time.

## TOML file

The file uses the same names in lower case, with these sections:

* top level: `listen`, `public_url`, `home_url`, `server_name`,
  `config_dir`, `database_url`, `web_dir`, `web_origins`, `mdns`,
  `log_format`, `allow_signups`, `admins`, `skip_email_verification`,
  `default_storage_bytes` and `storage_limit` (both in bytes),
  `hardcoded_ott` (fixed one-time codes per email, for testing);
* `[secrets]`: `jwt`, `encryption`, `hashing`;
* `[storage]`: `kind = "local"` with `path`, `public_url`; or `kind = "s3"`
  with `endpoint`, `public_endpoint`, `region`, `bucket`, `access_key`,
  `secret_key`, `secret_key_file`, `path_style`;
* `[smtp]`: `host`, `port`, `username`, `password`, `from`, `tls`;
* `[accounts]`: `public_albums_url`, `cast_url`, `accounts_url`,
  `family_url`, `public_locker_url`, `public_memories_url`,
  `webauthn_rp_id`, `webauthn_rp_origins`.
