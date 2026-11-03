# Crypto test vectors from the real Ente clients

These fixtures were produced by running the **unmodified client code** of
[ente-io/ente](https://github.com/ente-io/ente) at commit
`b2ed3dc93e49d6ac9831bf3550957816d3a0735d` (main, 2026-11-03; the same commit
is pinned in `tests/e2e/UPSTREAM_COMMIT`). They let server tests check
interoperability without a browser.

| File | Produced by | Client code exercised |
| --- | --- | --- |
| `srp.json` | `generator/` (Rust) | `rust/crates/accounts/src/auth/{srp,api}.rs`, `rust/crates/core/src/crypto/{argon,kdf}.rs` |
| `keys.json` | `generator/` (Rust) | `rust/crates/accounts/src/auth/key_gen.rs` (`generate_keys_with_strength`), `decrypt_keys_only`, `recovery_key_to_mnemonic` |
| `sealed_token.json` | `generator/` (Rust) | `rust/crates/core/src/crypto/sealed.rs` (`seal`/`open`), the auth-token payload check in `rust/crates/accounts/src/auth/payload.rs` |
| `libsodium.json` | `generator/libsodium-vectors.mts` (Node 22 + tsx) | `web/packages/base/crypto/libsodium.ts` with `libsodium-wrappers-sumo` 0.7.15 (the exact version pinned by `web/packages/base/package.json`) |

Where the web client's code lives at this commit: SRP, key derivation and
token opening moved from JS to Rust compiled to WebAssembly
(`ente-prelogin-wasm` = `rust/bindings/wasm/prelogin` → `rust/bindings/wasm/lib/src/prelogin.rs`
and `.../accounts/client.rs`), called from
`web/packages/accounts/services/{srp,user}.ts`. File, thumbnail and metadata
encryption, collection keys and sealed boxes for sharing still go through
`libsodium-wrappers-sumo` in `web/packages/base/crypto/libsodium.ts`. The
generator links the very same Rust crates natively (`ente-core`,
`ente-accounts`, `srp 0.7.0-rc.3`, `argon2 0.6.0`, `crypto_secretbox 0.1.1`,
with upstream's `Cargo.lock`), and runs `libsodium.ts` itself under Node.

## What the fixtures pin down

* **SRP** (`srp.json`): RFC 5054 4096-bit group, g = 5, SHA-256.
  Identity `I` = the `srpUserID` UUID string; password `P` = `loginSubKey`
  (first 16 bytes of `crypto_kdf_derive_from_key(32, 1, "loginctx", kek)`).
  `x = H(s | H(I | ":" | P))`, `k = H(N | PAD(g))`, `u = H(PAD(A) | PAD(B))`,
  `M1 = H(PAD(A) | PAD(B) | PAD(S))`, `K = H(PAD(S))`,
  `M2 = H(PAD(A) | M1 | K)`, PAD = left-pad to 512 bytes. The client sends
  `srpA` **already padded to 512 bytes** (standard base64) and verifies
  `srpM2`. The fixture has fixed `a` and `b`, so a server test can feed
  `srpA`/`srpM1` with the fixed `b` and must produce `srpB` and `srpM2`
  exactly. `goServerVector` reproduces upstream's own vector that was checked
  against museum's go-srp. The generator also ran the real `SrpSession`
  (random `a`) eight times against an independent server-side implementation
  of the formulas (`num-bigint`) and checks the client accepted every M2.
  Signup KDF parameters: the web app's "sensitive" derivation first tries
  memLimit 256 MiB / opsLimit 16 (then halves memory and doubles ops), so
  that is what a server normally receives.
* **Token** (`sealed_token.json`): when the API host is `api.ente.com`,
  `api.ente.in` **or `localhost`**, the client rejects a login whose sealed
  token plaintext is not `b"auth-token" || 32 random bytes`
  (`auth/payload.rs`, mirrors museum's `generateAuthToken`). The e2e harness
  runs on localhost, so this is enforced there.
* **Keys** (`keys.json`): a complete `keyAttributes` object as POSTed at signup,
  with every plaintext key, so tests can verify each box.
* **libsodium** (`libsodium.json`): secretbox (collection key under master
  key), file key under collection key, metadata / thumbnail / file contents
  via `crypto_secretstream_xchacha20poly1305` (single final chunk), a
  two-chunk stream (MESSAGE + FINAL tags), `crypto_box_seal`, keyed
  `crypto_generichash` (the server's email-hash primitive), the unkeyed
  BLAKE2b-512 file hash, and interactive argon2id. `srpCrossCheck` records that
  libsodium reproduces the Rust client's `kek` and `loginSubKey` in `srp.json`.

Random values (keys, nonces, headers) were drawn once; the fixtures are the
recorded outputs, so regenerating changes the bytes but not the structure.

## Tests that use them

* `server/tests/vectors_crypto.rs`: opens/verifies every fixture with the
  server's libsodium (`photovault_server::crypto` and raw `libsodium-sys`).
  `srp_fixture_kek_matches_libsodium_pwhash` is `#[ignore]`d (256 MiB × 16
  argon2id); run it with `cargo test --test vectors_crypto -- --ignored`.
* `server/tests/vectors_srp.rs`: checks the fixture's hash layout, then runs the
  server's SRP code (`photovault_server::users::srp`, used by login and password
  change, and `photovault_server::legacy::srp`, used by legacy recovery) on the
  real client's transcript: it must produce exactly `srpB` and `srpM2`.

## Checking that the clients accept tokens sealed by OUR server

Tests never spawn processes. To check the other direction (server seals,
client opens), write a server-sealed token and open it with both client
implementations (run manually or as a CI step):

```sh
mkdir -p /tmp/pv-vectors
PHOTOVAULT_VECTORS_OUT=/tmp/pv-vectors cargo test -p photovault-server --test vectors_crypto server_sealed_token
# Rust client crypto (what the web app runs as WebAssembly to open login tokens):
cargo run --manifest-path server/tests/vectors/generator/Cargo.toml -- verify-sealed /tmp/pv-vectors/server_sealed_token.json
# libsodium-wrappers-sumo through the web client's libsodium.ts:
npx tsx@4.19.2 server/tests/vectors/generator/libsodium-vectors.mts verify-sealed <ente>/web /tmp/pv-vectors/server_sealed_token.json
```

Both print `OK: ... auth-token payload: true` (checked 2026-11-03).

## Regenerating

```sh
# 1. Upstream checkout at the pinned commit with web deps installed
#    (tests/e2e/build-web.sh leaves one in ~/.cache/photovault-e2e/src-<commit>).
ln -sfn ~/.cache/photovault-e2e/src-b2ed3dc93e49 .upstream/ente   # .upstream/ is gitignored
export UPSTREAM_COMMIT=b2ed3dc93e49d6ac9831bf3550957816d3a0735d
# 2. Rust client vectors (srp.json, keys.json, sealed_token.json)
cargo run --manifest-path server/tests/vectors/generator/Cargo.toml -- generate server/tests/vectors
# 3. libsodium vectors (reads srp.json for the cross-check)
npx tsx@4.19.2 server/tests/vectors/generator/libsodium-vectors.mts generate .upstream/ente/web server/tests/vectors
```

`generator/` is a standalone crate (own `[workspace]`, upstream's
`Cargo.lock`); it is not part of the photovault workspace and never built by
`cargo test`.
