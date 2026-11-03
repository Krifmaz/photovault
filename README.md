# Photovault

A privacy-first, end-to-end encrypted photo platform. **Based on Ente (ente.com).**

Photovault keeps everything Ente does, exactly as Ente does it, and replaces
Ente's Go server ("museum") with a Rust server that the **unmodified** Ente
Photos apps (phone, web, desktop) can talk to. Photos stay end-to-end
encrypted: the server only ever stores ciphertext and never sees a photo,
thumbnail, file name, face or location.

> Photovault is an independent project and is not affiliated with or endorsed by Ente.

## Status

Section 1 (Rust server compatible with the stock Ente apps) is in progress.
The section 5 plugin SDK, host runtime and example plugins are in `plugins/sdk/`;
wiring them into the app forks follows once the forks exist.

## Quick start

```sh
docker build -t photovault:latest .
cp deploy/.env.example deploy/.env   # fill in hostnames and secrets
docker compose -f deploy/compose.sqlite.yaml --env-file deploy/.env up -d
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
| `plugins/sdk/` | WebAssembly plugin SDK, host runtime and example plugins |
| `deploy/` | Docker Compose files (SQLite or PostgreSQL) and example config |
| `docs/` | mdBook documentation (`mdbook build docs`) |
| `scripts/` | Maintenance scripts, including the upstream merge |

## Licence

AGPL-3.0, the same as Ente. See [LICENSE](LICENSE).
