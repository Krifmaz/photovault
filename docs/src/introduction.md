# Introduction

Photovault is a self-hosted, end-to-end encrypted photo platform.
**It is based on Ente ([ente.com](https://ente.com)).** Ente built the apps,
the cryptography and the protocol; Photovault keeps all of that exactly as
it is and replaces Ente's Go server ("museum") with a Rust server that the
**unmodified** Ente Photos apps for phone, web and desktop can talk to.

Photovault is an independent project. It is not affiliated with or endorsed
by Ente.

## What the server does and does not see

Every photo, video, thumbnail, file name, album name, face embedding and
location is encrypted on your device before upload. The server stores and
serves ciphertext plus the metadata needed to sync it (sizes, timestamps,
which encrypted file belongs to which encrypted collection). It never holds a
key that can decrypt your content, and nothing in Photovault is allowed to
need plaintext.

## Goals

* Drop-in compatibility with the stock Ente apps: same JSON field names and
  casing, same status codes, same error `code` strings as museum.
* Small and easy to run at home: one container under 40 MB, idle memory well
  under 64 MB, SQLite for single-user setups or PostgreSQL for families.
* Your disks or any S3: a plain directory (including a mergerfs pool) or any
  S3-compatible service.

## Licence

AGPL-3.0, the same as Ente.

## Where to go next

* [Deployment](deployment.md) to get a server running.
* [Configuration](configuration.md) for every setting.
* [Architecture](architecture.md) if you want to work on the code.
