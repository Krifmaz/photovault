# Permissions and sandbox

## Permissions

| Permission | Lets the plugin |
| --- | --- |
| `read:metadata` | Read file and album details: names, dates, GPS, type, captions, favourite/archive state, album membership |
| `read:thumbnails` | Read decrypted thumbnails |
| `read:originals` | Read decrypted originals |
| `write:tags` | Add tags |
| `write:albums` | Create albums you own and add or remove files in them |
| `write:trash` | Move files to the trash (recoverable for 30 days, as in Ente) |
| `write:export` | Ask the app to copy files into the export folder you picked. The plugin never sees the bytes or the real path |
| `network:<host>` | HTTP(S) to that host only. `*` and bare TLDs are not accepted |

You approve permissions on install and can revoke any of them later.
Revocation takes effect on the plugin's next host call. A call without the
permission fails with `permission_denied`; the plugin keeps running and can
handle it.

Network access, originals and trash are marked as sensitive on the install
screen.

## Sandbox

Plugins run in [Extism](https://extism.org) (Wasmtime) with:

- a memory cap (the plugin's request, clamped to the host maximum);
- a wall-clock timeout per hook call, plus an instruction (fuel) budget where
  available, so an infinite loop is stopped;
- **no file system**: no preopened directories, no environment variables;
- outbound HTTP only to the granted hosts, with a response size cap;
- no access to keys, the server session, or other plugins' data. Each plugin
  has its own small key/value store kept by the app.

## Signing and trust

A `.pvplugin` package holds the manifest, the Wasm module, the publisher's
Ed25519 public key and a signature (libsodium `crypto_sign_detached`) over a
BLAKE2b hash of both. The app refuses unsigned or tampered packages.

The first install of a plugin id pins its publisher key (trust on first use;
the install screen shows the key fingerprint). Updates signed by any other key
are refused, and downgrades are refused.
