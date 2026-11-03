// Generates libsodium fixtures by running the Ente web client's OWN crypto
// module (web/packages/base/crypto/libsodium.ts, which wraps
// libsodium-wrappers-sumo 0.7.15 as pinned in web/packages/base/package.json)
// under Node 22 via tsx. Also verifies a token sealed by OUR server.
//
//   tsx libsodium-vectors.mts generate <upstream>/web <out-dir>
//   tsx libsodium-vectors.mts verify-sealed <upstream>/web <server_sealed.json>
//
// See ../README.md for provenance and how to run.

import { readFileSync, writeFileSync } from "node:fs";
import { createRequire } from "node:module";
import path from "node:path";

const [mode, webDir, target] = process.argv.slice(2);
if (!mode || !webDir || !target) {
    console.error(
        "usage: tsx libsodium-vectors.mts generate|verify-sealed <upstream-web-dir> <out-dir|file>",
    );
    process.exit(2);
}

const require = createRequire(path.join(webDir, "packages/base/package.json"));
// eslint-disable-next-line @typescript-eslint/no-require-imports
const sodium = require("libsodium-wrappers-sumo");
const sodiumVersion = JSON.parse(
    readFileSync(require.resolve("libsodium-wrappers-sumo/package.json"), "utf8"),
).version as string;
const client = await import(
    path.join(webDir, "packages/base/crypto/libsodium.ts")
);
await sodium.ready;

const b64 = (u: Uint8Array) => sodium.to_base64(u, sodium.base64_variants.ORIGINAL);
const unb64 = (s: string) => sodium.from_base64(s, sodium.base64_variants.ORIGINAL);
const eq = (a: Uint8Array, b: Uint8Array) =>
    a.length === b.length && a.every((x, i) => x === b[i]);
const check = (cond: boolean, what: string) => {
    if (!cond) throw new Error(`self-check failed: ${what}`);
};

const verifySealed = (file: string) => {
    const v = JSON.parse(readFileSync(file, "utf8"));
    const pt = sodium.crypto_box_seal_open(
        unb64(v.encryptedToken),
        unb64(v.publicKey),
        unb64(v.secretKey),
    ) as Uint8Array;
    if (v.plaintext) check(eq(pt, unb64(v.plaintext)), "plaintext matches");
    const prefix = new TextEncoder().encode("auth-token");
    const wellFormed = pt.length === 42 && eq(pt.slice(0, 10), prefix);
    console.log(
        `OK: libsodium-wrappers-sumo ${sodiumVersion} opened the server-sealed token (${pt.length} bytes, auth-token payload: ${wellFormed})`,
    );
};

const generate = async (outDir: string) => {
    const commit = process.env.UPSTREAM_COMMIT ?? "unknown";
    const provenance = {
        generator: "server/tests/vectors/generator/libsodium-vectors.mts",
        upstream: "https://github.com/ente-io/ente",
        upstreamCommit: commit,
        clientCode: "web/packages/base/crypto/libsodium.ts (+ types.ts)",
        library: `libsodium-wrappers-sumo ${sodiumVersion}`,
        node: process.version,
    };

    // Account-level keys.
    const masterKey = unb64(await client.generateKey());
    const { publicKey, privateKey } = sodium.crypto_box_keypair();

    // Collection key, encrypted with the master key (POST /collections
    // encryptedKey/keyDecryptionNonce).
    const collectionKey = await client.generateKey();
    const collBox = await client.encryptBox(collectionKey, masterKey);
    check(
        (await client.decryptBox(collBox, masterKey)) === collectionKey,
        "collection key box",
    );

    // File key (secretstream key) encrypted with the collection key
    // (POST /files encryptedKey/keyDecryptionNonce).
    const fileKey = await client.generateBlobOrStreamKey();
    const fileKeyBox = await client.encryptBox(fileKey, collectionKey);

    // Metadata via secretstream (single FINAL chunk).
    const metadata = {
        title: "IMG_0001.jpg",
        creationTime: 1730000000000000,
        modificationTime: 1730000000000000,
        fileType: 0,
        hash: "placeholder-hash",
    };
    const encMeta = await client.encryptMetadataJSON(metadata, fileKey);
    check(
        JSON.stringify(await client.decryptMetadataJSON(encMeta, fileKey)) ===
            JSON.stringify(metadata),
        "metadata",
    );

    // Thumbnail via encryptBlobBytes (single FINAL chunk).
    const thumb = sodium.randombytes_buf(300) as Uint8Array;
    const encThumb = await client.encryptBlobBytes(thumb, fileKey);

    // File contents: a small single-chunk file and a 2-chunk stream built
    // with initChunkEncryption/encryptStreamChunk (as the uploader does for
    // large files; real chunks are 4 MiB, here we use 1000 + 24 bytes to
    // keep the fixture small: the format is chunk-size agnostic except that
    // decryptors read 4 MiB + 17 byte chunks).
    const fileData = sodium.randombytes_buf(1000) as Uint8Array;
    const encFile = await client.encryptStreamBytes(fileData, fileKey);
    const { decryptionHeader: streamHeader, pushState } =
        await client.initChunkEncryption(fileKey);
    const c1 = await client.encryptStreamChunk(fileData, pushState, false);
    const c2 = await client.encryptStreamChunk(
        fileData.slice(0, 24),
        pushState,
        true,
    );

    // Sealed box (collection sharing / public key encryption).
    const sealedPlain = sodium.randombytes_buf(32) as Uint8Array;
    const sealed = await client.boxSeal(b64(sealedPlain), b64(publicKey));

    // Hashes.
    const hashKey = sodium.randombytes_buf(32) as Uint8Array;
    const email = "vectors@example.com";
    const keyedHash = sodium.crypto_generichash(
        32,
        new TextEncoder().encode(email),
        hashKey,
    ) as Uint8Array;
    const hs = await client.chunkHashInit();
    await client.chunkHashUpdate(hs, fileData);
    const fileHash = await client.chunkHashFinal(hs);

    // Interactive argon2id via the client's deriveInteractiveKey
    // (used e.g. for public-link passwords).
    const derived = await client.deriveInteractiveKey("public link password");

    // Cross-check the Rust (ente-core) SRP fixture's KDF with libsodium.
    let srpCrossCheck: string = "srp.json not present";
    try {
        const srp = JSON.parse(readFileSync(path.join(outDir, "srp.json"), "utf8"));
        const kek = sodium.crypto_pwhash(
            32,
            srp.inputs.password,
            unb64(srp.inputs.kekSalt),
            srp.inputs.opsLimit,
            srp.inputs.memLimit,
            sodium.crypto_pwhash_ALG_ARGON2ID13,
        ) as Uint8Array;
        check(eq(kek, unb64(srp.derived.kek)), "srp.json kek == libsodium pwhash");
        const sub = sodium.crypto_kdf_derive_from_key(32, 1, "loginctx", kek) as Uint8Array;
        check(
            eq(sub.slice(0, 16), unb64(srp.derived.loginSubKey)),
            "srp.json loginSubKey == libsodium kdf",
        );
        srpCrossCheck = "ok: kek and loginSubKey in srp.json match libsodium";
    } catch (e) {
        if (e instanceof Error && e.message.startsWith("self-check")) throw e;
        srpCrossCheck = `skipped: ${String(e)}`;
    }

    const out = {
        description:
            "libsodium outputs produced by the Ente web client's own crypto module. Every field is standard base64 unless noted. The server only ever stores these ciphertexts.",
        provenance,
        srpCrossCheck,
        secretbox: {
            what: "encryptBox: crypto_secretbox_easy, random 24-byte nonce (collection key under master key)",
            key: b64(masterKey),
            plaintext: collectionKey,
            encryptedData: collBox.encryptedData,
            nonce: collBox.nonce,
        },
        fileKey: {
            what: "file key (crypto_secretstream key) encrypted with the collection key, as sent in POST /files",
            collectionKey,
            fileKey,
            encryptedKey: fileKeyBox.encryptedData,
            keyDecryptionNonce: fileKeyBox.nonce,
        },
        metadata: {
            what: "encryptMetadataJSON: JSON.stringify -> UTF-8 -> secretstream xchacha20poly1305, one TAG_FINAL chunk",
            key: fileKey,
            json: JSON.stringify(metadata),
            encryptedData: encMeta.encryptedData,
            decryptionHeader: encMeta.decryptionHeader,
        },
        thumbnail: {
            what: "encryptBlobBytes: secretstream, one TAG_FINAL chunk",
            key: fileKey,
            plaintext: b64(thumb),
            encryptedData: b64(encThumb.encryptedData),
            decryptionHeader: b64(encThumb.decryptionHeader),
        },
        file: {
            what: "encryptStreamBytes (<= 4 MiB: one TAG_FINAL chunk)",
            key: fileKey,
            plaintext: b64(fileData),
            encryptedData: b64(encFile.encryptedData),
            decryptionHeader: encFile.decryptionHeader,
            blake2b512: fileHash,
        },
        fileStream: {
            what: "initChunkEncryption + encryptStreamChunk: chunk 1 TAG_MESSAGE (1000 B), chunk 2 TAG_FINAL (24 B)",
            key: fileKey,
            decryptionHeader: streamHeader,
            chunks: [b64(c1), b64(c2)],
            plaintexts: [b64(fileData), b64(fileData.slice(0, 24))],
        },
        sealedBox: {
            what: "boxSeal: crypto_box_seal",
            publicKey: b64(publicKey),
            secretKey: b64(privateKey),
            plaintext: b64(sealedPlain),
            sealed,
        },
        generichash: {
            what: "crypto_generichash(32, email, key): the server's keyed email hash primitive",
            key: b64(hashKey),
            message: email,
            hash: b64(keyedHash),
        },
        argon2id: {
            what: "deriveInteractiveKey: crypto_pwhash ARGON2ID13, interactive limits",
            password: "public link password",
            salt: derived.salt,
            opsLimit: derived.opsLimit,
            memLimit: derived.memLimit,
            key: derived.key,
        },
    };
    const file = path.join(outDir, "libsodium.json");
    writeFileSync(file, JSON.stringify(out, null, 2) + "\n");
    console.log(`wrote ${file}`);
};

if (mode === "generate") await generate(target);
else if (mode === "verify-sealed") verifySealed(target);
else {
    console.error(`unknown mode ${mode}`);
    process.exit(2);
}
