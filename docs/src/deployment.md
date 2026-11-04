# Deployment

Photovault ships as one container image, `ghcr.io/krifmaz/photovault:edge`:

* one release binary on `gcr.io/distroless/cc-debian12:nonroot` (no shell,
  no package manager), running as uid/gid `65532`;
* the web app, served on the same port as the API (`PHOTOVAULT_WEB_DIR=/app/web`);
* two volumes: `/data` (encrypted objects) and `/config` (the SQLite
  database `photovault.db` and `keys.toml`);
* `HEALTHCHECK` runs `photovault healthcheck`, which probes `/healthz`
  (a database round trip) without curl;
* graceful shutdown on `SIGTERM` (what `docker stop` sends).

SQLite is the default database. PostgreSQL is optional.

## Prepare the folders

The container runs as uid 65532, so it must own its two host folders:

```sh
mkdir -p /drivepool/photovault /opt/photovault
chown -R 65532:65532 /drivepool/photovault /opt/photovault
```

Keep `/config` (the database) on a local disk such as `/opt/photovault`,
not on mergerfs, NFS or SMB: databases rely on locking and `fsync` that
those layers do not always provide. Objects are fine on mergerfs (see
[Storage backends](storage.md#durability-and-mergerfs)).

## On the home network

[`deploy/compose.yaml`](https://github.com/Krifmaz/photovault/blob/main/deploy/compose.yaml):

```yaml
name: photovault
services:
  photovault:
    image: ghcr.io/krifmaz/photovault:edge
    restart: unless-stopped
    ports:
      - "8095:8080"
    environment:
      PHOTOVAULT_PUBLIC_URL: http://10.0.1.7:8095
    volumes:
      - /drivepool/photovault:/data   # encrypted photos
      - /opt/photovault:/config       # database + keys (local disk)
    read_only: true
    security_opt: [no-new-privileges:true]
    cap_drop: [ALL]
```

```sh
docker compose -f deploy/compose.yaml up -d
docker compose -f deploy/compose.yaml logs -f
```

Replace `10.0.1.7` with the address of the Docker host. Open
`http://10.0.1.7:8095` for the web app, and enter the same address in the
phone app. The file also has commented lines for the server name, quotas,
SMTP and the `photovault-mdns` service that lets the apps find the server
without typing an address ([LAN discovery](discovery.md)).

The phone apps accept plain `http://` for a LAN address. Anything reachable
from the internet should use HTTPS through a reverse proxy.

## On a domain

[`deploy/compose.domain.yaml`](https://github.com/Krifmaz/photovault/blob/main/deploy/compose.domain.yaml)
puts the server behind a reverse proxy on the external network `proxy_net`,
with two hostnames:

| Setting | Example | Used for |
| --- | --- | --- |
| `PHOTOVAULT_PUBLIC_URL` | `https://photos.example.com` | API and web app; what you enter in the apps. |
| `PHOTOVAULT_STORAGE_PUBLIC_URL` | `https://storage.photos.example.com` | Signed photo upload and download URLs. |
| `PHOTOVAULT_HOME_URL` | `http://10.0.1.7:8095` | LAN address. At home the apps use it when it answers as the same server. |

See [HTTPS behind a reverse proxy](deployment/reverse-proxy.md) for the
proxy settings. The storage hostname must not go through Cloudflare's proxy.

## First start

The server checks its setup before it opens the port. On the first start it
creates `/config/keys.toml` and `/config/photovault.db`, then logs a line
such as:

```
photovault 0.1.0 listening on 0.0.0.0:8080 (storage local, free 3800.0 GB, email: codes written to this log, signups closed)
```

If a check fails it prints one line that ends in the fix, waits 30 seconds
and exits with code 78:

```
photovault: cannot write /data: owned by uid 0, running as uid 65532; fix: chown -R 65532:65532 <host folder mounted at /data>
```

With `restart: unless-stopped` Docker starts it again, so it retries every
30 seconds or so until you fix the problem.

## Create your account

Sign-ups are off by default. Create accounts with the
[admin CLI](admin-cli.md):

```sh
docker exec photovault photovault admin create-user you@example.com
```

`photovault` here is the container name. Compose calls it
`photovault-photovault-1` unless you add `container_name: photovault`;
`docker compose exec photovault photovault admin …` works either way.

It prints a code. In the app choose *Create account*, enter the email and
the code, and pick a password there. The server never sees the password.

Without SMTP, the codes the app asks for later (for example on a new
device) are written to the log: `docker logs photovault`.

## Point the apps at your server

* **Photovault apps:** enter the address on the *Connect to your server*
  screen, scan the QR code from the web app's settings, or pick the server
  found on the LAN.
* **Stock Ente apps:** tap the welcome screen seven times, open
  *Developer settings*, and enter the same address. On the web and desktop
  apps the same dialog opens from the login screen.

The apps check the address with `/photovault/info` (Photovault apps) or
`/ping` (Ente's apps).

## PostgreSQL

[`deploy/compose.postgres.yaml`](https://github.com/Krifmaz/photovault/blob/main/deploy/compose.postgres.yaml)
adds a `postgres:16-alpine` container and sets `PHOTOVAULT_DATABASE_URL`.
Use it for many active users or if you already run Postgres. Keys still
live in `/config/keys.toml` unless you set them in the environment.

## Moving from older images

**Old SQLite path.** Older images kept SQLite at
`/db/photovault.sqlite`. The server will not start an empty database in
`/config` while that file is mounted, and prints:

```
photovault: found the database of an older install at /db/photovault.sqlite; fix: mount the config folder at /config and move that file to /config/photovault.db, or set PHOTOVAULT_DATABASE_URL=sqlite:///db/photovault.sqlite?mode=rwc
```

Stop the container, move the file (for example
`mv /opt/photovault/photovault.sqlite /opt/photovault/photovault.db`),
mount that folder at `/config` instead of `/db`, and start again. If your
old compose file set `PHOTOVAULT_JWT_SECRET`, `PHOTOVAULT_ENCRYPTION_KEY`
and `PHOTOVAULT_HASHING_KEY`, keep them, or paste them into
`/config/keys.toml` as `jwt_secret`, `encryption_key` and `hashing_key`.
New keys would log everyone out and make stored email addresses unreadable.

**Postgres to SQLite.** A one-time copy, run while `/config` is mounted:

```sh
docker compose -f compose.postgres.yaml stop photovault
docker compose -f compose.postgres.yaml run --rm photovault \
  migrate-to-sqlite --from postgres://photovault:PASSWORD@db:5432/photovault
```

It copies every table into `/config/photovault.db` (it refuses if that file
exists; `--to` picks another path), prints the row counts, and saves the
keys from the environment into `/config/keys.toml` if that file does not
exist yet. Then remove `PHOTOVAULT_DATABASE_URL`, the key variables,
`depends_on` and the `db` service, and start the stack again. Keep the
Postgres folder until you have checked that everything is there.

## Backups

Back up:

* `/config`: `photovault.db` and `keys.toml`. Copy the database with
  `sqlite3 photovault.db ".backup /backup/photovault.db"` on the host, or
  stop the container first. With Postgres use `pg_dump` and keep `keys.toml`.
* `/data` (or the bucket). Everything there is already encrypted.

## Next

* [Docker on Proxmox LXC with Portainer](deployment/proxmox-portainer.md)
* [HTTPS behind a reverse proxy](deployment/reverse-proxy.md)
* [Optional mTLS device certificates](deployment/mtls.md)
* [LAN discovery](discovery.md)
