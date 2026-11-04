# Photovault

A privacy-first, end-to-end encrypted photo platform. **Based on Ente (ente.com).**

Photovault keeps everything Ente does, exactly as Ente does it, and replaces
Ente's Go server ("museum") with a Rust server that the **unmodified** Ente
Photos apps (phone, web, desktop) can talk to. Photos stay end-to-end
encrypted: the server only ever stores ciphertext and never sees a photo,
thumbnail, file name, face or location.

> Photovault is an independent project and is not affiliated with or endorsed by Ente.

## Status

Section 1 (Rust server compatible with the stock Ente apps) is complete:

* All 306 museum routes are implemented ([API matrix](docs/src/api-compatibility.md)), on Postgres or SQLite, with local-disk or S3 storage.
* The unmodified Ente Photos web app passes the end-to-end suite in `tests/e2e/` (21 pass, 1 skipped: the web app has no Legacy screens), the same result as Ente's own server; the desktop app smoke test passes.
* The mobile app's API usage is audited call by call ([audit](docs/src/mobile-api-audit.md)) with tests in `server/tests/mobile_compat.rs`; a manual phone checklist is in `tests/e2e/README.md`.
* SRP and crypto are checked against vectors produced by the real Ente client code (`server/tests/vectors/`).

## Quick start

```sh
mkdir -p /drivepool/photovault /opt/photovault
chown -R 65532:65532 /drivepool/photovault /opt/photovault
# edit PHOTOVAULT_PUBLIC_URL and the two host folders in deploy/compose.yaml
docker compose -f deploy/compose.yaml up -d
```

Objects can live on a local directory (a mergerfs pool is fine) or any
S3-compatible service. See [docs/src/deployment.md](docs/src/deployment.md)
for HTTPS, reverse proxies and the Proxmox/Portainer setup, and
[UPSTREAM.md](UPSTREAM.md) for how the app forks track Ente.

## Repository layout

| Path | Contents |
| --- | --- |
| `server/` | Rust server (replaces Ente's Go "museum") |
| `node/` | Rust compute node (optional, later) |
| `importers/` | Immich and iCloud import tools (later) |
| `mobile/`, `web/`, `desktop/` | Light forks of Ente's apps (later) |
| `plugins/sdk/` | Plugin SDK and examples (later) |
| `deploy/` | Docker Compose files (LAN, domain, PostgreSQL) and example config |
| `docs/` | mdBook documentation (`mdbook build docs`) |
| `scripts/` | Maintenance scripts, including the upstream merge |

## Licence

AGPL-3.0, the same as Ente. See [LICENSE](LICENSE).
