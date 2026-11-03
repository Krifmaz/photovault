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

## Repository layout

| Path | Contents |
| --- | --- |
| `server/` | Rust server (replaces Ente's Go "museum") |
| `node/` | Rust compute node (optional, later) |
| `importers/` | Immich and iCloud import tools (later) |
| `mobile/`, `web/`, `desktop/` | Light forks of Ente's apps (later) |
| `plugins/sdk/` | Plugin SDK and examples (later) |
| `docs/` | mdBook documentation |
| `scripts/` | Maintenance scripts, including the upstream merge |

## Licence

AGPL-3.0, the same as Ente. See [LICENSE](LICENSE).
