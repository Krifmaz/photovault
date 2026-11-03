# Deployment

Photovault ships as one container image:

* a single release binary on `gcr.io/distroless/cc-debian12:nonroot`
  (glibc only: no shell, no package manager), running as uid/gid `65532`;
* `HEALTHCHECK` runs `photovault healthcheck`, which probes `/healthz`
  (database round trip) without needing curl;
* volumes `/data` (objects, local backend) and `/db` (SQLite file);
* graceful shutdown on `SIGTERM` (what `docker stop` sends).

## Build the image

```sh
git clone https://github.com/Krifmaz/photovault
cd photovault
docker build -t photovault:latest .
```

Or let Docker fetch the source: `docker build -t photovault:latest https://github.com/Krifmaz/photovault.git`.

## Choose a database

| | SQLite | PostgreSQL |
| --- | --- | --- |
| Good for | one person or a small family | several active users |
| Moving parts | none (file in `/db`) | a `postgres:16` container |
| Compose file | `deploy/compose.sqlite.yaml` | `deploy/compose.yaml` |

Keep the SQLite file (or the Postgres data directory) on a local disk such as
`/opt/photovault`, **not** on mergerfs, NFS or SMB: databases rely on locking
and `fsync` semantics those layers do not always provide. Objects are fine on
mergerfs (see [Storage backends](storage.md#durability-and-mergerfs)).

## Secrets

Generate three keys once and keep them with your backups:

```sh
for k in JWT_SECRET ENCRYPTION_KEY HASHING_KEY; do
  echo "PHOTOVAULT_$k=$(head -c 32 /dev/urandom | base64)"
done
```

Losing `PHOTOVAULT_ENCRYPTION_KEY`/`PHOTOVAULT_HASHING_KEY` makes stored email
addresses unreadable (photos stay safe: they are encrypted with keys only your
devices hold, but you need your account to reach them).

## Point the apps at your server

The stock apps have a hidden *Developer settings* dialog for a custom server
(Ente's self-hosting guide calls this "connecting to a custom server"):

* **Mobile:** tap the welcome screen seven times, open *Developer settings*,
  enter `https://photos.example.com`.
* **Web and desktop:** open the same *Developer settings* dialog from the
  login screen and enter the same URL. A self-built web app can instead bake
  it in with `NEXT_PUBLIC_ENTE_ENDPOINT=https://photos.example.com`.

The URL is your `public_url`; the apps check it by calling `/ping`.

## Backups

Back up three things: the database (`pg_dump` or a copy of the SQLite file
taken with `sqlite3 photovault.sqlite ".backup …"`), the object directory (or
bucket), and the secrets. Everything in the object store is already
encrypted.

## Next

* [Docker on Proxmox LXC with Portainer](deployment/proxmox-portainer.md)
* [HTTPS behind a reverse proxy](deployment/reverse-proxy.md)
* [Optional mTLS device certificates](deployment/mtls.md)
