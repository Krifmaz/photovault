# Admin CLI

`photovault admin …` works on the database directly, with the same
configuration as the server, so it works before any account exists. The
image has no shell; run it with `docker exec`:

```sh
docker exec photovault photovault admin list-users
```

`photovault` is the container name. Compose calls it
`photovault-photovault-1` unless you set `container_name: photovault`;
`docker compose exec photovault photovault admin …` works either way.

Users are named by email or numeric user id.

## End-to-end encryption and passwords

Photos are encrypted on your devices with a key that only your password (or
your recovery key) can open. The server never sees the password or the key.
So the server cannot choose a password for someone or reset it. Instead:

* `create-user` makes the account and prints a code; the person picks
  their password in the app.
* `reset-password` prints a code and the steps to set a new password with
  the recovery key in the app.

## Accounts

| Command | What it does |
| --- | --- |
| `admin create-user <email> [--quota 500GB] [--admin]` | Create an account and print a code valid for 24 hours. Works with sign-ups off. |
| `admin reset-password <user>` | Print a code and the "Forgot password" steps. |
| `admin reset-password <user> --erase --yes` | Delete the account and create an empty one with the same email. **All of its photos are lost.** |
| `admin list-users` | Id, email, quota, usage in bytes, admin flag. |
| `admin delete-user <user> --yes` | Delete the account (data is removed after the recovery window). |
| `admin mark-admin <user> [--revoke]` | Grant or remove access to the `/admin/*` API. |
| `admin disable-2fa <user> [--passkeys]` | Turn off two-factor authentication; `--passkeys` also removes all passkeys. |

### `create-user`

```sh
docker exec photovault photovault admin create-user anna@example.com --quota 200GB
```

```
user 3: created anna@example.com
next: in the app choose "Create account" with anna@example.com, enter code 123456 (valid 24 hours), then choose a password
```

Sign-ups can stay off: this account already exists, so the app's
*Create account* flow finishes it. The app then generates the keys and the
recovery key on the device. `--quota` defaults to
`PHOTOVAULT_DEFAULT_QUOTA`.

### `reset-password`

```sh
docker exec photovault photovault admin reset-password anna@example.com
```

The person enters their email in the app, taps *Forgot password*, enters
the printed code if asked, then their 24-word recovery key, and chooses a
new password. Their photos stay.

Without the recovery key there is no way back into the photos. To give the
person a fresh, empty account with the same email:

```sh
docker exec photovault photovault admin reset-password anna@example.com --erase --yes
```

## Storage and sign-ups

| Command | What it does |
| --- | --- |
| `admin quota <user> <size\|none>` | Set a quota, e.g. `50GB` (binary units). `none` removes it; the free space is then the limit. Aliases: `set-quota`, `set-storage`. |
| `admin signups [on\|off]` | Open or close sign-up. With no value, show the state. Stored in the database, takes effect at once and overrides `PHOTOVAULT_ALLOW_SIGNUPS`. |

## Sign-in codes

| Command | What it does |
| --- | --- |
| `admin pending-ott <email>` | Show unexpired one-time codes for an email. Useful without SMTP. |
| `admin add-ott <email> <code> [--app photos\|auth\|locker] [--minutes 60]` | Add a code (at least 6 characters) that the email can sign in with. |

## Other commands

| Command | What it does |
| --- | --- |
| `photovault serve` | Run the server (the default). |
| `photovault migrate` | Apply database migrations and exit. |
| `photovault healthcheck [--url URL]` | Probe `/healthz`; non-zero exit unless it answers 2xx. Used by the image's `HEALTHCHECK`. |
| `photovault gen-keys` | Print three fresh keys as `PHOTOVAULT_*=` lines. |
| `photovault announce [--server URL] [--port N]` | Announce a server on the LAN via mDNS. See [LAN discovery](discovery.md). |
| `photovault migrate-to-sqlite [--from postgres://…] [--to FILE]` | Copy a Postgres database into a new SQLite file (default `/config/photovault.db`) and save the current keys to `keys.toml` if it is missing. See [Deployment](deployment.md#moving-from-older-images). |

`migrate-to-sqlite` is not under `admin`; run it in a one-off container:

```sh
docker compose run --rm photovault migrate-to-sqlite --from postgres://photovault:PASSWORD@db:5432/photovault
```

If the keys are invalid or `/config` is not writable, admin commands print
the same one-line message as the server's startup checks and exit with
code 78.
