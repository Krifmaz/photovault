# Storage backends

The apps never send file bodies through the API. Like museum, the server
hands out short-lived **signed URLs** and the apps `PUT`/`GET` encrypted
objects directly against them. Object keys look like `<userID>/<uuid>`; the
contents are always ciphertext.

Two backends implement the same `ObjectStore` trait
(`server/src/storage/mod.rs`): **local** and **S3**.

## Local directory (`kind = "local"`)

Objects are files under `PHOTOVAULT_STORAGE_PATH` (`/data` in the image). The server signs and serves its own
URLs under `/storage/<key>` (HMAC-SHA256 with `secrets.jwt`, bound to the
operation, key, expiry and, for v2 uploads, the exact length and MD5), and
speaks the subset of S3 the apps use:

| Request | Behaviour |
| --- | --- |
| `PUT /storage/<key>?…` | Stream the body to disk. When the URL bound a `Content-MD5` (the apps' v2 upload URLs) the request must send that header and the body must match: wrong/missing header → `403 SignatureDoesNotMatch`, wrong body → `400 BadDigest`, wrong length → `403`/`400 IncompleteBody`. A `Content-MD5` sent without being bound is still verified. Response `ETag` is the quoted MD5 hex, like S3. |
| `PUT …?uploadId=…&partNumber=N` | Store one multipart part, with the same checks; returns the part `ETag`. |
| `POST …?uploadId=…` | `CompleteMultipartUpload` XML from the app. Parts must be ascending and their ETags must match, then the parts are **streamed** into the object (never buffered in memory). A retry after success answers `200`. |
| `GET`/`HEAD /storage/<key>?…` | Stream the object. Single `Range` requests give `206`, unsatisfiable ones `416`. |

Errors are S3-style XML (`<Error><Code>…</Code></Error>`).

### Durability, and mergerfs

* Every write goes to a hidden temp file **in the same directory** as the
  final object (`.<name>.pv-tmp-<random>`), is `fsync`ed, renamed over the
  target, and then the directory is `fsync`ed. A crash leaves either the old
  state or the complete new object, never a torn file.
* On a mergerfs pool the temp file and its target share a directory, so the
  rename is a plain same-directory `rename(2)` that mergerfs performs on the
  branch holding the temp file. Use a create policy that keeps a new file and
  its directory together, e.g. `category.create=mfs` (or `epmfs` if you
  pre-create the per-user directories on every branch). If your FUSE setup
  refuses `fsync` on directories, Photovault carries on (the file data itself
  is still `fsync`ed).
* In-progress multipart uploads live in `<path>/.multipart/<uploadId>/`.
  Completion copies the parts into the object file and deletes the upload
  directory; for a 10 GB video that is 10 GB of sequential I/O, done once.
* Hourly housekeeping deletes multipart uploads untouched for 8 days (part
  URLs are valid for 7) and temp files older than a day left by a crash.

### Separate storage hostname

`PHOTOVAULT_STORAGE_PUBLIC_URL` (default: `PHOTOVAULT_PUBLIC_URL`) is the base of every signed URL.
Pointing it at a second hostname, e.g. `https://storage.example.com`, that
reaches the same container lets you give uploads different proxy settings
(no body size limit, no buffering, long timeouts) or bypass a CDN with upload
limits. See [HTTPS behind a reverse proxy](deployment/reverse-proxy.md).

## Storage numbers

The apps show "X of Y used". Photovault fills in real numbers instead of a
plan size:

* **used**: the bytes the account stores. In a family it is the family's
  total, as in Ente.
* **free**: what the backend can still take.
  * Local: free space on the disk behind `PHOTOVAULT_STORAGE_PATH`, read with
    `statvfs` (space available to normal users) and cached for a few seconds.
  * S3: there is no such call, so it is `PHOTOVAULT_STORAGE_LIMIT` minus
    everything stored on the server. Without `PHOTOVAULT_STORAGE_LIMIT` it is
    unlimited.
  * `PHOTOVAULT_STORAGE_LIMIT` also caps a local disk, for example to keep
    part of a shared pool free.
* **quota**: the account's quota (`photovault admin quota`), or
  `PHOTOVAULT_DEFAULT_QUOTA` for new accounts. None by default.
* **limit** = `min(quota, used + free)`. A side that is "none" or unlimited
  is left out.

When there is no limit at all (S3 without `PHOTOVAULT_STORAGE_LIMIT` and no
quota), Ente's endpoints report **100 TiB** (`109951162777600` bytes), so
the stock apps still show a number.

The values go into Ente's existing fields (`/users/details/v2`:
`usage` and `subscription.storage`; `/billing/subscription`), so the stock
apps show them without changes.

`GET /photovault/usage` (signed in, `X-Auth-Token`) returns the same numbers
explicitly:

```json
{ "usedBytes": 52428800000, "limitBytes": 3800000000000, "freeBytes": 3747571200000,
  "unlimited": false, "limitSource": "disk", "quotaBytes": null }
```

`limitSource` says which side set the limit: `quota`, `default-quota`,
`disk`, `storage-limit` or `unlimited`. When `unlimited` is `true`,
`limitBytes` and `freeBytes` are `null`.

## S3-compatible services (`kind = "s3"`)

Works with AWS S3, Backblaze B2, Wasabi, Cloudflare R2, Garage, MinIO,
Ceph RGW and others that implement SigV4. Photovault signs requests itself
(`server/src/storage/s3/sigv4.rs`, tested against AWS's published vectors):

* **Presigned for the apps:** `PUT` (binding `content-length` and
  `content-md5` as signed headers when the app supplied them, like museum's
  v2 upload URLs), `GET`, `UploadPart` (also binding per-part length/MD5) and
  `CompleteMultipartUpload`. URLs are capped at SigV4's 7-day maximum.
* **Server side:** `CreateMultipartUpload`, `AbortMultipartUpload`,
  `HeadObject`, `DeleteObject`, `CopyObject` (multipart copy above 5 GiB),
  and small `PutObject`/`GetObject`. Connection errors and 5xx are retried
  with backoff.

TLS uses rustls with the `ring` provider and the system trust store; no
OpenSSL. Plain `http://` endpoints (a Garage/MinIO on the same Docker
network) need no TLS at all.

### Bucket CORS

The web app uploads from the browser straight to the bucket, so the bucket
must allow it. Example (AWS/B2/MinIO JSON form):

```json
[{
  "AllowedOrigins": ["https://web.photos.example.com"],
  "AllowedMethods": ["GET", "HEAD", "PUT", "POST"],
  "AllowedHeaders": ["*"],
  "ExposeHeaders": ["ETag"],
  "MaxAgeSeconds": 3000
}]
```

`ExposeHeaders: ETag` is essential: the web app reads each part's `ETag` to
complete multipart uploads.

### Internal vs public endpoint

If the server reaches the bucket at `http://garage:3900` but the apps must use
`https://s3.example.com`, set `endpoint` to the former and `public_endpoint`
to the latter. Presigned URLs embed the public host in their signature, so the
reverse proxy in front of the S3 service must pass the `Host` header through
unchanged.

### Garage example

```toml
[storage]
kind = "s3"
endpoint = "http://garage:3900"
public_endpoint = "https://s3.photos.example.com"
region = "garage"
bucket = "photovault"
access_key = "GK..."
secret_key_file = "/run/secrets/s3_secret"
```

`scripts/test-s3-garage.sh` runs the S3 integration tests against a
throwaway Garage container.

### Cleaning up abandoned uploads

S3 keeps parts of an unfinished multipart upload (and bills for them) until
it is aborted. Photovault records upload ids and aborts stale ones; as a
belt-and-braces measure add a bucket lifecycle rule "abort incomplete
multipart uploads after 8 days".
