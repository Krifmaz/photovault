// Generates node.json: crypto vectors for photovault-import produced by
// Ente's web crypto functions.
//
// The functions in the "Ente web" section below are copied from
// ente-io/ente web/packages/base/crypto/libsodium.ts (only the TypeScript
// types and the two tiny ente-utils helpers were replaced), and run on
// libsodium-wrappers-sumo 0.7.15, the exact version Ente's web app pins.
//
// Usage: npm ci && node gen.mjs ../../node.json

import { createHash } from "node:crypto";
import { writeFileSync } from "node:fs";
import sodium from "libsodium-wrappers-sumo";

// ---- ente-utils stand-ins ------------------------------------------------

const mergeUint8Arrays = (as) => {
    const out = new Uint8Array(as.reduce((n, a) => n + a.length, 0));
    let off = 0;
    for (const a of as) {
        out.set(a, off);
        off += a.length;
    }
    return out;
};
const ensureArrayBufferBacked = (a) => a;
const streamEncryptionChunkSize = 4 * 1024 * 1024; // base/crypto/types.ts

// ---- Ente web (base/crypto/libsodium.ts) -----------------------------------

const toB64 = async (input) => {
    await sodium.ready;
    return sodium.to_base64(input, sodium.base64_variants.ORIGINAL);
};

const fromB64 = async (input) => {
    await sodium.ready;
    return ensureArrayBufferBacked(
        sodium.from_base64(input, sodium.base64_variants.ORIGINAL),
    );
};

const bytes = async (bob) => (typeof bob == "string" ? fromB64(bob) : bob);

const encryptBox = async (data, key) => {
    await sodium.ready;
    const nonce = sodium.randombytes_buf(sodium.crypto_secretbox_NONCEBYTES);
    const encryptedData = sodium.crypto_secretbox_easy(
        await bytes(data),
        nonce,
        await bytes(key),
    );
    return {
        encryptedData: await toB64(encryptedData),
        nonce: await toB64(nonce),
    };
};

const encryptBlobBytes = async (data, key) => {
    await sodium.ready;

    const keyBytes = await bytes(key);
    const initPushResult =
        sodium.crypto_secretstream_xchacha20poly1305_init_push(keyBytes);
    const [pushState, header] = [initPushResult.state, initPushResult.header];

    const pushResult = sodium.crypto_secretstream_xchacha20poly1305_push(
        pushState,
        await bytes(data),
        null,
        sodium.crypto_secretstream_xchacha20poly1305_TAG_FINAL,
    );
    return {
        encryptedData: ensureArrayBufferBacked(pushResult),
        decryptionHeader: ensureArrayBufferBacked(header),
    };
};

const encryptStreamBytes = async (data, key) => {
    await sodium.ready;

    const keyBytes = await bytes(key);
    const initPushResult =
        sodium.crypto_secretstream_xchacha20poly1305_init_push(keyBytes);
    const [pushState, header] = [initPushResult.state, initPushResult.header];
    let bytesRead = 0;
    let tag = sodium.crypto_secretstream_xchacha20poly1305_TAG_MESSAGE;

    const encryptedChunks = [];

    while (tag !== sodium.crypto_secretstream_xchacha20poly1305_TAG_FINAL) {
        let chunkSize = streamEncryptionChunkSize;
        // Equality marks the last full-size chunk as final.
        if (bytesRead + chunkSize >= data.length) {
            chunkSize = data.length - bytesRead;
            tag = sodium.crypto_secretstream_xchacha20poly1305_TAG_FINAL;
        }

        const buffer = data.slice(bytesRead, bytesRead + chunkSize);
        bytesRead += chunkSize;
        const pushResult = sodium.crypto_secretstream_xchacha20poly1305_push(
            pushState,
            buffer,
            null,
            tag,
        );
        encryptedChunks.push(pushResult);
    }
    return {
        encryptedData: mergeUint8Arrays(encryptedChunks),
        decryptionHeader: await toB64(header),
    };
};

const chunkHashInit = async () => {
    await sodium.ready;
    return sodium.crypto_generichash_init(
        null,
        sodium.crypto_generichash_BYTES_MAX,
    );
};

const chunkHashUpdate = async (hashState, chunk) => {
    await sodium.ready;
    sodium.crypto_generichash_update(hashState, chunk);
};

const chunkHashFinal = async (hashState) => {
    await sodium.ready;
    const hash = sodium.crypto_generichash_final(
        hashState,
        sodium.crypto_generichash_BYTES_MAX,
    );
    return toB64(hash);
};

const boxSeal = async (data, publicKey) => {
    await sodium.ready;
    return toB64(
        sodium.crypto_box_seal(await fromB64(data), await fromB64(publicKey)),
    );
};

const deriveKey = async (passphrase, salt, opsLimit, memLimit) => {
    await sodium.ready;
    return await toB64(
        sodium.crypto_pwhash(
            sodium.crypto_secretbox_KEYBYTES,
            sodium.from_string(passphrase),
            await fromB64(salt),
            opsLimit,
            memLimit,
            sodium.crypto_pwhash_ALG_ARGON2ID13,
        ),
    );
};

// ---- deterministic inputs (shared with the Go generator and Rust tests) ----

/** byte i = high byte of (i + seed) * 2654435761 mod 2^32 */
const pattern = (seed, n) => {
    const out = new Uint8Array(n);
    for (let i = 0; i < n; i++) out[i] = Math.imul(i + seed, 2654435761) >>> 24;
    return out;
};

/** n bytes of SHA-256("label/0") || SHA-256("label/1") || ... */
const fixed = (label, n) => {
    const parts = [];
    for (let c = 0; parts.length * 32 < n; c++)
        parts.push(createHash("sha256").update(`${label}/${c}`).digest());
    return new Uint8Array(Buffer.concat(parts).subarray(0, n));
};

const sha256b64 = (b) => createHash("sha256").update(b).digest("base64");

// ---- generate --------------------------------------------------------------

await sodium.ready;
const b64 = (b) => sodium.to_base64(b, sodium.base64_variants.ORIGINAL);
const MiB4 = streamEncryptionChunkSize;
const v = {
    generator: "importers/tests/vectors/gen/node (Ente web base/crypto/libsodium.ts on libsodium-wrappers-sumo 0.7.15)",
    sodium_version: sodium.sodium_version_string(),
};

v.argon = [];
for (const [pw, ops, mem, label] of [
    ["correct horse battery staple", 2, 64 * 1024 * 1024, "argon-salt-1"],
    ["pässwörd ✓", 1, 8192, "argon-salt-2"],
    ["", 3, 16 * 1024 * 1024, "argon-salt-3"],
]) {
    const salt = b64(fixed(label, 16));
    v.argon.push({ password: pw, salt, ops_limit: ops, mem_limit: mem, key: await deriveKey(pw, salt, ops, mem) });
}

// libsodium's own KDF (what Ente's clients use for the SRP login sub-key).
v.login_key = [];
for (let i = 0; i < 3; i++) {
    const kek = fixed(`kek-${i}`, 32);
    const sub = sodium.crypto_kdf_derive_from_key(32, 1, "loginctx", kek);
    v.login_key.push({ kek: b64(kek), login_key: b64(sub.slice(0, 16)) });
}

v.secretbox = [];
for (const [i, n] of [0, 1, 32, 1000].entries()) {
    const key = fixed(`box-key-${i}`, 32);
    const pt = pattern(100 + i, n);
    const { encryptedData, nonce } = await encryptBox(pt, key);
    v.secretbox.push({ key: b64(key), nonce, plaintext: b64(pt), ciphertext: encryptedData });
}

v.sealed = [];
for (const [i, n] of [0, 32, 200].entries()) {
    const kp = sodium.crypto_box_seed_keypair(fixed(`seal-seed-${i}`, 32));
    const pt = pattern(200 + i, n);
    const ct = await boxSeal(b64(pt), b64(kp.publicKey));
    v.sealed.push({ public_key: b64(kp.publicKey), secret_key: b64(kp.privateKey), plaintext: b64(pt), ciphertext: ct });
}

v.blob = [];
for (const [i, n] of [0, 1, 500].entries()) {
    const key = fixed(`nblob-key-${i}`, 32);
    const pt = pattern(300 + i, n);
    const { encryptedData, decryptionHeader } = await encryptBlobBytes(pt, key);
    v.blob.push({ key: b64(key), header: b64(decryptionHeader), plaintext: b64(pt), ciphertext: b64(encryptedData) });
}

// Small streams: full ciphertext. Large streams (multi-chunk, chunk-boundary
// sizes): only the SHA-256 of Ente's ciphertext, which the Rust test
// reproduces byte for byte from the same key and header.
v.stream_small = [];
for (const [i, n] of [0, 1, 1000, 65536].entries()) {
    const key = fixed(`stream-key-${i}`, 32);
    const seed = 400 + i;
    const { encryptedData, decryptionHeader } = await encryptStreamBytes(pattern(seed, n), key);
    v.stream_small.push({ key: b64(key), header: decryptionHeader, seed, len: n, ciphertext: b64(encryptedData) });
}
v.stream_large = [];
for (const [i, n] of [MiB4 - 1, MiB4, MiB4 + 1, 2 * MiB4, 2 * MiB4 + 12345].entries()) {
    const key = fixed(`lstream-key-${i}`, 32);
    const seed = 500 + i;
    const { encryptedData, decryptionHeader } = await encryptStreamBytes(pattern(seed, n), key);
    v.stream_large.push({
        key: b64(key),
        header: decryptionHeader,
        seed,
        len: n,
        ciphertext_len: encryptedData.length,
        ciphertext_sha256: sha256b64(encryptedData),
    });
}

// Content hash as Ente's upload computes it: BLAKE2b-512 fed 4 MiB at a time.
v.hash = [];
for (const [seed, n] of [[600, 0], [601, 3], [602, 1000], [603, MiB4 + 5]]) {
    const data = pattern(seed, n);
    const st = await chunkHashInit();
    for (let off = 0; off < n; off += MiB4) await chunkHashUpdate(st, data.subarray(off, off + MiB4));
    v.hash.push({ seed, len: n, hash: await chunkHashFinal(st) });
}
v.hash_abc = b64(sodium.crypto_generichash(64, sodium.from_string("abc"), null));

writeFileSync(process.argv[2] ?? "node.json", JSON.stringify(v, null, 2) + "\n");
