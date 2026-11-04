# Docker on Proxmox LXC with Portainer

This is the reference setup:

* Proxmox host with a **mergerfs** pool mounted at `/drivepool`.
* A Debian LXC running Docker, managed by **Portainer**.
* Encrypted objects in `/drivepool/photovault`, the database and keys in
  `/opt/photovault` on the LXC's local disk.
* Optional: a reverse proxy container on an external Docker network
  `proxy_net`, for access through a domain.

## 1. Give the LXC the pool

On the Proxmox host, bind-mount the pool into the container (CT 105 here):

```sh
pct set 105 -mp0 /drivepool,mp=/drivepool
```

For Docker inside an LXC enable nesting (`features: nesting=1,keyctl=1`).

## 2. Create the folders

The Photovault image runs as uid/gid **65532**. Create and hand over the
folders **inside the LXC**:

```sh
mkdir -p /drivepool/photovault /opt/photovault
chown -R 65532:65532 /drivepool/photovault /opt/photovault
```

In an unprivileged LXC, uid `N` inside is host uid `100000 + N`, so on the
host these files belong to uid **165532**. If the LXC cannot `chown` on the
pool (it shows up as `nobody:nogroup`), run it on the Proxmox host instead:
`chown -R 165532:165532 /drivepool/photovault`.

### mergerfs settings

Objects are written to a temp file next to their final name and renamed, so
use a create policy that keeps a new file in its directory's branch, e.g.
`category.create=mfs` with path preservation off, and leave
`cache.files=off` (or `partial`) so `fsync` reaches the disks. Never put
`/config` on the pool.

## 3. Deploy the stack in Portainer

*Stacks → Add stack → Web editor*, paste
[`deploy/compose.yaml`](https://github.com/Krifmaz/photovault/blob/main/deploy/compose.yaml),
change `10.0.1.7` to the LXC's address, and deploy. No environment
variables or secrets are needed: the keys are created in
`/opt/photovault/keys.toml` on the first start. Back that file up.

For a domain use
[`deploy/compose.domain.yaml`](https://github.com/Krifmaz/photovault/blob/main/deploy/compose.domain.yaml)
(create `proxy_net` once with `docker network create proxy_net` if your proxy
has not). For Postgres use `deploy/compose.postgres.yaml` and set
`POSTGRES_PASSWORD` under *Environment variables*.

The container runs read-only, with no capabilities and no privilege
escalation.

## 4. Read the startup checks

Open *Containers → photovault-photovault-1 → Logs*. A good start looks like:

```
WARN Generated new keys in /config/keys.toml. Back up this file; it cannot be recreated.
INFO photovault 0.1.0 listening on 0.0.0.0:8080 (storage local, free 3800.0 GB, email: codes written to this log, signups closed)
```

(The first line appears only on the first start.) If a check fails you see
one line instead, ending in the fix:

```
photovault: cannot write /data: owned by uid 0, running as uid 65532; fix: chown -R 65532:65532 <host folder mounted at /data>
photovault: config folder /config does not exist; fix: mount a host folder there (volumes: - <host folder>:/config) or create it
photovault: cannot write /config: read-only file system; fix: mount the volume read-write
photovault: cannot write /data: no space left on device; fix: free space on the disk behind it
```

The container waits 30 seconds, exits with code 78, and Docker restarts it.
Fix the problem (usually the `chown` above) and it starts on the next try.

## 5. Create your account

Sign-ups are off. In the LXC:

```sh
docker exec photovault-photovault-1 photovault admin create-user you@example.com
```

Then choose *Create account* in the app with that email and the printed
code. See [Admin CLI](../admin-cli.md). Portainer's container console does
not work: the image has no shell.

Until SMTP is set up, the codes the apps ask for are in the log:
`Verification code for <email>: <code>`.

## 6. Check health

Portainer shows the container as *healthy* once `/healthz` answers. Metrics
are at `http://<container>:8080/metrics` from inside the Docker network; do
not expose them publicly (see [reverse proxy](reverse-proxy.md)).
