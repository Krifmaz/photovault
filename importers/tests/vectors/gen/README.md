# Crypto test-vector generators

The JSON files one level up are produced by **Ente's own code**, so the Rust
tests in `importers/` check our crypto against the real apps, not against
our reading of them. `cargo test -p photovault-import` reads only the
committed JSON; Go and Node are needed only to regenerate the vectors or to
run the `#[ignore]`d reverse-direction tests.

Vectors were last generated from `ente-io/ente` at commit
`b2ed3dc93e49d6ac9831bf3550957816d3a0735d`.

| File | Generator | Ente code used |
| --- | --- | --- |
| `go.json` | `go/` | CLI `cli/internal/crypto` (Argon2id, login sub-key, secretstream blob), museum's SRP library `github.com/ente/go-srp`, and the CLI's `github.com/kong/go-srp` |
| `node.json` | `node/` | `web/packages/base/crypto/libsodium.ts` functions (copied verbatim into `gen.mjs`) on `libsodium-wrappers-sumo` 0.7.15, the version Ente's web app pins |

## What is covered

- Argon2id KEK derivation (three parameter sets), checked against both Go and web.
- SRP login sub-key (`crypto_kdf_derive_from_key(32, 1, "loginctx")[..16]`).
- Secretbox, sealed box, and single-message FINAL blobs made by Ente.
- File streams made by Ente's `encryptStreamBytes`: small ones (0, 1, 1000,
  65536 bytes) with full ciphertext, and large ones at chunk boundaries
  (4 MiB − 1, 4 MiB, 4 MiB + 1, 8 MiB, 8 MiB + 12345) as key, header,
  length and SHA-256 of the ciphertext. Ente's 4 MiB chunk size is fixed, so
  the multi-chunk ciphertexts are not committed: the Rust unit test rebuilds
  them byte for byte from the recorded key and header, checks the hash, then
  decrypts them.
- BLAKE2b-512 content hashes fed 4 MiB at a time, as Ente's upload does.
- SRP-6a: museum's `go-srp` server and client with fixed secrets, including
  cases where `A`, `B` or `S` start with a zero byte (pins the padding).

Plaintexts are deterministic: byte `i` of the pattern with seed `s` is the
high byte of `(i + s) * 2654435761 mod 2^32`. Fixed keys and salts are
`SHA-256("label/0") || SHA-256("label/1") ...` truncated. Nonces and headers
are random (Ente's code draws them), so regenerating changes the ciphertexts
but not the test logic.

## Regenerating

You need Go (the CLI module asks for Go 1.26; with `GOTOOLCHAIN=auto` an older
`go` downloads it) and Node 22 with npm.

```sh
# 1. An ente-io/ente checkout; link its cli/ directory into the Go generator.
git clone https://github.com/ente-io/ente /path/to/ente
ln -s /path/to/ente/cli importers/tests/vectors/gen/go/ente-cli

# 2. Go vectors (the module path sits under github.com/ente/cli so Go's
#    internal-package rule lets it import cli/internal/crypto).
cd importers/tests/vectors/gen/go
go run . gen ../../go.json

# 3. Node vectors.
cd ../node
npm ci
node gen.mjs ../../node.json
```

Then run `cargo test -p photovault-import`. Keep the JSON files small (well
under 200 KB each).

## Reverse direction (our output, Ente decrypts)

With the `ente-cli` link in place:

```sh
cargo test -p photovault-import --test crypto_vectors -- --ignored
```

- `ente_cli_decrypts_our_output` writes our streams (including 4 MiB, 4 MiB + 1
  and multi-chunk files), blob, secretbox, sealed box and Argon2 output to a
  temp dir and runs `go run . verify <file>`, which decrypts them with the
  Ente CLI's crypto using Ente's chunking.
- `srp_login_against_museum_go_srp_server` logs in with our `SrpClient`
  against `go run . srp-server`, museum's go-srp server with a random secret,
  and checks that a wrong password is rejected.

Set `GO=/path/to/go` if `go` is not on `PATH`. The tests start Go with
`std::process::Command` arguments, never through a shell.
