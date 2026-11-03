# Docker on Proxmox LXC with Portainer

This is the reference setup:

* Proxmox host with a **mergerfs** pool mounted at `/drivepool`.
* A Debian LXC running Docker, managed by **Portainer**.
* A reverse proxy container (Nginx Proxy Manager, Traefik, Caddy…) attached
  to an external Docker network called **`proxy_net`**.
* Objects in `/drivepool/photovault`, the database in `/opt/photovault`
  (SQLite file, or a Postgres container's data directory).

## 1. Give the LXC the pool

On the Proxmox host, bind-mount the pool into the container (CT 105 here):

```sh
pct set 105 -mp0 /drivepool,mp=/drivepool
```

For Docker inside an LXC enable nesting (`features: nesting=1,keyctl=1`).

### Unprivileged LXC and file ownership

The Photovault image runs as uid/gid **65532**. In an unprivileged LXC,
container uid `N` is host uid `100000 + N`, so files Photovault writes on the
pool belong to host uid **165532**. Create and hand over the directories
**inside the LXC**:

```sh
mkdir -p /drivepool/photovault /opt/photovault
chown 65532:65532 /drivepool/photovault /opt/photovault
```

If the LXC cannot `chown` on the pool (it shows up as `nobody:nogroup`),
do it on the host instead: `chown 165532:165532 /drivepool/photovault`.
With Postgres, its container chowns `/opt/photovault/postgres` itself (uid 70
in `postgres:16-alpine`).

### mergerfs settings

Objects are written to a temp file next to their final name and renamed, so
use a create policy that keeps a new file in its directory's branch, e.g.
`category.create=mfs` with path-preservation off, and leave
`cache.files=off` (or `partial`) so `fsync` reaches the disks. Never put the
database on the pool.

## 2. Create the network once

```sh
docker network create proxy_net   # skip if your proxy already made it
```

## 3. Build the image in the LXC

```sh
docker build -t photovault:latest https://github.com/Krifmaz/photovault.git
```

## 4. Deploy the stack in Portainer

*Stacks → Add stack → Web editor*, paste
[`deploy/compose.yaml`](https://github.com/Krifmaz/photovault/blob/main/deploy/compose.yaml)
(PostgreSQL) or `deploy/compose.sqlite.yaml` (SQLite), then add the
variables from `deploy/.env.example` under *Environment variables*:

| Variable | Example |
| --- | --- |
| `PHOTOS_HOST` | `photos.example.com` |
| `STORAGE_HOST` | `storage.photos.example.com` (optional) |
| `PHOTOVAULT_JWT_SECRET` | `head -c 32 /dev/urandom \| base64` |
| `PHOTOVAULT_ENCRYPTION_KEY` | 〃 |
| `PHOTOVAULT_HASHING_KEY` | 〃 |
| `POSTGRES_PASSWORD` | Postgres stack only |

The compose files publish no ports: the proxy reaches the container as
`photovault:8080` over `proxy_net`, and Postgres sits on a private `internal`
network. The container runs read-only, without capabilities, with a 256 MB
memory cap (idle use is far lower).

## 5. Create your account

With `PHOTOVAULT_ALLOW_SIGNUPS=false`, create accounts with the
[admin CLI](../admin-cli.md) from Portainer's container console, or allow
sign-ups briefly, register in the app, and switch it back.

Login codes are written to the container log until SMTP is configured:
*Containers → photovault → Logs*.

## 6. Check health

Portainer shows the container as *healthy* once `/healthz` answers. Metrics
are at `http://photovault:8080/metrics` from inside `proxy_net`; do not expose
them publicly (see [reverse proxy](reverse-proxy.md)).
