# Mobile app API audit

A static audit of every HTTP call the Photos mobile app makes, compared with
Photovault's routes. The app could not be run here, so this was done by reading
the source of ente-io/ente at commit `b2ed3dc93e49d6ac9831bf3550957816d3a0735d`
(2026-11-03):

* `mobile/apps/photos/lib` (Dio gateways and services),
* the shared Dart packages under `mobile/packages` that Photos depends on
  (`legacy`, `contacts`, `feature_flag`, `install_source`, `sharing`,
  `account_deletion`),
* the Rust core those packages call through flutter_rust_bridge
  (`rust/crates/{core,accounts,legacy,contacts}`). Login, SRP, 2FA, passkeys,
  account deletion, Legacy (trusted contacts and kits) and contacts all go
  through Rust `reqwest` with strict `serde` models, not Dio.

Every row below names the test in `server/tests/mobile_compat.rs` that sends
the request the way the app does and checks only what the app reads back.
Tests that fail today are `#[ignore = "mobile-compat: …"]`. To run them:
`cargo test --test mobile_compat -- --include-ignored`.

Status values: **matches** means a test passes; **mismatch** means the server
answers but the app would break, and there is an ignored test for it;
**missing** means there is no route; **matches (existing)** means it is
covered by an older test suite, named in the notes; **by review** means
the handler was read side by side with museum but has no test.

## Result

Only one mismatch was found on SQLite and Postgres:

| # | Call | Mismatch | Test |
| --- | --- | --- | --- |
| 1 | `POST /push/token` | The app always sends `"apnsToken": null` (and `"notificationKey": null`) on Android. museum's Go `string` field decodes `null` as `""`. Our `PushTokenRequest.apns_token: String` with `#[serde(default)]` rejects an explicit `null`, so the app gets **400 `{}`** and push registration silently fails. Fix: `Option<String>` (or `deserialize_with` that maps null → `""`). | `push_token_with_nulls` (ignored) |

One route is missing, and it is missing in museum too. No test was added for it:

| Call | Notes |
| --- | --- |
| `POST {endpoint}/anonymous/feedback` `{feedback, type}` (public Dio, no token) | Sent only from the Stripe cancellation survey (`StripeSubscriptionPage`) and from `packages/accounts`. museum has no such route either. On a self-hosted server the call 404s and the app ignores the error. No action needed unless we want to collect the feedback. |

**General risk, worth a sweep.** Go's `encoding/json` accepts `null` for any
non-pointer field and leaves it at the zero value. serde rejects `null` for
`String`, `i64` and `bool` even when `#[serde(default)]` is set, because
`default` only covers a *missing* field. The push token handler is the only
case this audit proved with the app. Any handler that deserialises a Go value
type into a non-`Option` serde field can fail the same way when a client
sends `null`.

## Headers the app sends

| Header | Sent by | Value |
| --- | --- | --- |
| `X-Auth-Token` | `EnteRequestInterceptor` on `enteDio` (API base URL only); Rust `Auth::User`; download/thumbnail Dio, set by hand | session token |
| `X-Client-Package` | every Dio `BaseOptions`; Rust `ApiConfig.client_package` | `io.ente.photos` (Android F-Droid/independent builds use other suffixes) |
| `X-Client-Version` | same | app version, e.g. `1.2.3` |
| `User-Agent` | `ua_client_hints` value on Dio; set explicitly on the Rust core | device UA |
| `x-request-id` | `EnteRequestInterceptor` | `droid-…` / `ios-…` |
| `X-Auth-Access-Token` (+ `X-Auth-Access-Token-JWT`) | public album / memory share calls | link token (+ password JWT) |
| `Content-MD5`, `Content-Length`, `Content-Type: application/octet-stream` | object and multipart part PUTs (`_s3Dio`) | base64 MD5 |
| `Range: bytes=a-b` | chunked downloads to the signed URL | |

Transport details that matter for compatibility:

* **Dio `post(path)` with no `data`** sends no body and no `Content-Type`. The
  app does this for `/users/logout`, `/users/two-factor/disable`,
  `/family/create`, `/collections/leave/{id}`,
  `/storage-bonus/referral-claim?code=`, `/billing/stripe/cancel-subscription`
  and `/billing/stripe/activate-subscription`. Tested; all accepted.
* `ApiResponseInterceptor` turns error responses whose body is not JSON into
  `UnexpectedApiResponseException`. It keeps the status code, so `{}` and empty
  error bodies are fine.
* The Rust core's API client has **redirects disabled**
  (`redirect::Policy::none`) and parses every success body strictly with
  serde. Required fields must be present, `Vec`s must not be `null`, and enum
  strings must match exactly.
* The core enforces auth payload prefixes (`auth-token` + 32 bytes for the
  login token, `delete-jwt` on the deletion challenge, HMAC `proof` instead of
  `secret` in 2FA removal) only when the API host is `api.ente.com`,
  `api.ente.in` or **`localhost`**. Photovault already implements all three
  (`src/users/mod.rs`, `routes/users/account.rs`, `routes/users/twofactor.rs`),
  so a server reached as `http://localhost:8080` (for example over
  `adb reverse`) works.
* The Cloudflare worker and upload proxy URLs (`files.ente.com`,
  `thumbnails.ente.com`, `kUploadProxyEndpoint`) are used only when the
  endpoint is the production one. A custom endpoint always goes direct.

## Every call

### Endpoint check, flags, events

| Caller | Request | App reads | Our route | Status / test |
| --- | --- | --- | --- | --- |
| developer settings `_ping` (plain Dio) | `GET {endpoint}/ping` | `message == "pong"` | `health::ping` | matches: `ping_has_message_and_id` |
| Rust `Api::ping` | `GET /ping` | `message`, `id` (both required `String`) | same | matches: same test |
| `FlagService` | `GET /remote-store/feature-flags` | `RemoteFlags.fromMap` (all optional) | `remote_store` | matches: `flags_billing_and_bonus_views` |
| `FlagService.setFlag` | `POST /remote-store/update {key, value}` | status 200 | `remote_store` | matches (existing `users_services`) |
| `InstallSourceService` | `POST /events` (install, public) / `POST /events/user` (`sign_up`/`log_in`) with `app`, `platform` added | nothing | `misc` | matches (existing `users_services`) |

### Account (Dio `UsersGateway` + Rust `ente_accounts`)

| Caller | Request | App reads | Our route | Status / test |
| --- | --- | --- | --- | --- |
| core `get_srp_attributes` | `GET /users/srp/attributes?email=` | `attributes{srpUserID uuid, srpSalt, memLimit u64, opsLimit u32, kekSalt, isEmailMFAEnabled}`; 404 → "SRP not configured" | `users::srp` | matches: `srp_attributes_unknown_email_is_404`, existing `users_srp_client` |
| core `create_srp_session` | `POST /users/srp/create-session {srpUserID, srpA}` | `sessionID` uuid, `srpB` | same | matches (existing `users_srp_client`) |
| core `verify_srp_session` | `POST /users/srp/verify-session {srpUserID, sessionID, srpM1}` | `AuthResponse` (`id`, `keyAttributes?`, `encryptedToken?`, `twoFactorSessionID[V2]?`, `passkeySessionID?`, `srpM2`, and `accountsUrl` **required when `passkeySessionID` is set**); 401 = wrong password | same | matches (existing `users_flow`, `users_passkeys`) |
| core `send_otp` | `POST /users/ott {email, purpose, mobile?}` | codes `USER_NOT_REGISTERED` 404, `USER_ALREADY_REGISTERED` 409, `USER_SIGNUP_INCOMPLETE` 404; 403 for `change` | `users::ott` | matches: `ott_login_unknown_email`, existing `users_flow` |
| core `verify_email` | `POST /users/verify-email {email, ott, source?}` | `AuthResponse`; 401/410/429 | same | matches (existing) |
| core `set_user_key_attributes` | `PUT /users/attributes {keyAttributes}` | status | same | matches (existing) |
| core / `UsersGateway.setRecoveryKey` | `PUT /users/recovery-key {masterKeyEncryptedWithRecoveryKey, masterKeyDecryptionNonce, recoveryKeyEncryptedWithMasterKey, recoveryKeyDecryptionNonce}` | status (only sent when unset; museum answers 500 when already set) | same | matches: `recovery_key_passkey_recovery_and_legacy_thumbnail` |
| core `setup_srp` / `complete_srp_setup` / `update_srp_and_key_attributes` | `POST /users/srp/setup`, `/complete {setupID, srpM1}`, `/update {setupID, srpM1, updatedKeyAttr, logOutOtherDevices}` | `setupID` uuid, `srpB`, `srpM2` | same | matches (existing `users_srp_client`) |
| core `get_session_validity` | `GET /users/session-validity/v2` | `hasSetKeys`, `keyAttributes?`; 401 = invalid | same | matches: `session_validity_shape` |
| core `email` / `UsersGateway.getUserDetails` | `GET /users/details/v2?memoryCount=true\|false` | `email` String, `usage` int, `subscription{productID, storage, expiryTime, paymentProvider, price, period, attributes?}`, `familyData?{members[], storage, expiryTime, adminBonus?}`, `profileData?`, `bonusData?{storageBonuses[]}`, `fileCount?`, `storageBonus?`, `sharedCollectionsCount?` | `users::account::details` | matches: `user_details_v2_shape`, `family_data_shape_after_create` |
| `UsersGateway.changeEmail` | `POST /users/change-email {email, ott}` | status (403/409 handled) | same | matches (by review; needs a mailed OTT) |
| `UsersGateway.getPublicKey` | `GET /users/public-key?email=` | `publicKey`; 404 → null | same | matches: `public_key_lookups` |
| `UsersGateway.getPublicKeys` | `POST /users/public-keys {emails ≤10}` | `publicKeys` aligned, each `as String`; 404 → per-email fallback | same | matches: `public_key_lookups` |
| `UsersGateway.getActiveSessions` | `GET /users/sessions` | `sessions[]{token, isCurrent, creationTime, ip, ua, prettyUA, lastUsedTime}` | same | matches: `sessions_list_and_terminate` |
| `UsersGateway.terminateSession` | `DELETE /users/session?token=` | status (current session → 400, as in museum) | same | matches: same test |
| core `logout` / `UsersGateway.logout` | `POST /users/logout` (`{}` from Rust; no body from Dio) | status | same | matches: `logout_without_body` |
| core `get_two_factor_status` / gateway | `GET /users/two-factor/status` | `status` bool | `twofactor::status` | matches: `two_factor_disable_and_status_without_body` |
| core `setup_two_factor` | `POST /users/two-factor/setup {}` | `secretCode`, `qrCode` | `twofactor::setup` | matches: `two_factor_views` |
| core `enable_two_factor` | `POST /users/two-factor/enable {code, encryptedTwoFactorSecret, twoFactorSecretDecryptionNonce}` | 401 = wrong code | same | matches (existing `users_flow`) |
| `UsersGateway.disableTwoFactor` | `POST /users/two-factor/disable` (no body) | status | same | matches: `two_factor_disable_and_status_without_body` |
| core `verify_totp` | `POST /users/two-factor/verify {sessionID, code}` | `AuthResponse`; 401 wrong, 404/410 expired, 429 | same | matches: `totp_verify_unknown_session` |
| core `get_two_factor_recovery` | `GET /users/two-factor/recover?sessionID&twoFactorType=totp\|passkey` | `encryptedSecret`, `secretDecryptionNonce`; 404 expired; 400 (passkey) = not configured | same | matches (existing `users_flow`) |
| core `remove_two_factor` | `POST /users/two-factor/remove {sessionID, secret? \| proof?, twoFactorType}` | `{id, keyAttributes, encryptedToken}` | same | matches (existing) |
| core `get_two_factor_recovery_status` | `GET /users/two-factor/recovery-status` | `isPasskeyRecoveryEnabled` | same | matches: `two_factor_views` |
| core `configure_passkey_recovery` | `POST /users/two-factor/passkeys/configure-recovery {secret, userSecretCipher, userSecretNonce}` | status | same | matches: `recovery_key_passkey_recovery_and_legacy_thumbnail` |
| core `check_passkey_status` | `GET /users/two-factor/passkeys/get-token?sessionID=` | `AuthResponse`; **400 = keep polling**, 404/410 = expired | same | matches: `passkey_get_token_unknown_session_is_not_400` |
| core `get_accounts_token` | `GET /users/accounts-token` | `accountsUrl`, `accountsToken` | `users::accounts_token` | matches: `payment_token_email_mfa_accounts_token` |
| `UsersGateway.updateEmailMFA` | `PUT /users/email-mfa {isEnabled}` | status | same | matches: same test |
| `UsersGateway.getPaymentToken` | `GET /users/payment-token` | `paymentToken` | same | matches: same test |
| core deletion | `GET /users/deletion-summary` → `photosAndVideosCount`, `authenticatorCodesCount`, `lockerRecordsCount` (required i64); `GET /users/delete-challenge` → `encryptedChallenge` (sealed, optionally `delete-jwt`-prefixed); `DELETE /users/delete {challenge, reasonCategory, feedback?}` | | `users::account` | summary matches: `account_deletion_views`; challenge/delete matches (existing `users_flow`) |
| `UsersGateway.sendFeedback` | `POST /anonymous/feedback` | ignored | none | **missing**, and missing in museum too (see above) |

### Family

| Caller | Request | App reads | Our route | Status / test |
| --- | --- | --- | --- | --- |
| `createFamily` | `POST /family/create` (no body) | status; 402 on a free plan, as in museum | `family` | matches: `family_create_and_leave_without_body`, `family_data_shape_after_create` |
| `inviteFamilyMember` | `POST /family/add-member {email}` | status | same | matches (existing `users_services`) |
| `removeFamilyMember` / `revokeFamilyInvite` | `DELETE /family/remove-member/{id}`, `/family/revoke-invite/{id}` | status | same | remove: matches (existing `users_services`); revoke-invite: by review |
| `updateFamilyMemberStorage` | `POST /family/modify-storage {id, storageLimit: int \| null}` | status | same | matches: `family_modify_storage_null_limit` (null accepted) |
| `leaveFamilyPlan` | `DELETE /family/leave` | status | same | matches: `leave_family_when_not_member` |

### Collections, sharing, links

| Caller | Request | App reads | Our route | Status / test |
| --- | --- | --- | --- | --- |
| `createCollection` | `POST /collections {encryptedKey, keyDecryptionNonce, encryptedName, nameDecryptionNonce, type, attributes{encryptedPath?, pathDecryptionNonce?, version}, magicMetadata?}` | `collection` map | `collections` | matches: every test using `album()` |
| `getCollection` | `GET /collections/{id}` | `collection` | same | matches: `file_edits_and_magic_metadata` |
| `getAll` | `GET /collections/v2?sinceTime&source` | `collections[]` (`isDeleted` omitempty, read as `?? false`) | same | matches: `upload_then_collection_diff_loop` |
| `getDiff` (sync loop) | `GET /collections/v2/diff?collectionID&sinceTime` | `diff[]{id, collectionID, updationTime, isDeleted, ownerID, encryptedKey, keyDecryptionNonce, collectionAddedAt?, file/thumbnail/metadata.decryptionHeader, metadata.encryptedData, info?.fileSize, magicMetadata?, pubMagicMetadata?}`, `hasMore` bool; loops on `max(updationTime)` | same | matches: `upload_then_collection_diff_loop` (re-poll returns empty) |
| `deleteCollection` | `DELETE /collections/v3/{id}?keepFiles&collectionID` | status | same | matches: `delete_collection_v3_query` |
| `renameCollection` | `POST /collections/rename {collectionID, encryptedName, nameDecryptionNonce}` | status | same | matches: `file_edits_and_magic_metadata` |
| `leaveCollection` | `POST /collections/leave/{id}` (no body) | status | same | matches: `leave_collection_without_body` |
| `update[Public\|Sharee]MagicMetadata` | `PUT /collections/{magic,public-magic,sharee-magic}-metadata {id, magicMetadata{version,count,data,header}}` | status; 409 → refetch and retry | same | matches: `file_edits_and_magic_metadata`, `collaborator_suggest_delete_and_sharee_metadata` |
| `joinViaLink` | `POST /collections/join-link {collectionID, encryptedKey}` + `X-Auth-Access-Token` | status | same | matches: `join_album_via_link` |
| `fetchPendingRemovalActions` / `fetchDeleteSuggestions` | `GET /collection-actions/{pending-remove,delete-suggestions}` | `actions[]` | `collection_actions` | matches: `collection_actions_views` |
| `rejectDeleteSuggestions` | `POST /collection-actions/reject-delete-suggestions {fileIDs}` | status | same | matches: `collection_actions_views`, `collaborator_suggest_delete_and_sharee_metadata` |
| `addFiles` / `restoreFiles` / `moveFiles` | `POST /collections/{add,restore,move}-files {collectionID \| to/fromCollectionID, files[{id, encryptedKey, keyDecryptionNonce}]}` | status | same | matches: `file_edits_and_magic_metadata` (add); existing `library_flow` (move, restore) |
| `removeFiles` | `POST /collections/v3/remove-files {collectionID, fileIDs}` | status 200 (owner's own files → 400, as in museum) | same | matches: same test |
| `suggestDelete` | `POST /collections/suggest-delete {collectionID, fileIDs}` (owner/admin, on other people's files) | status 200 | same | matches: `collaborator_suggest_delete_and_sharee_metadata` |
| `copyFiles` | `POST /files/copy {dstCollectionID, srcCollectionID, files[]}` | `oldToNewFileIDMap` (string keys → int) | `files` | matches: `copy_shared_file_into_own_album` |
| `getSharees` / `share` / `unshare` | `GET /collections/sharees?collectionID`; `POST /collections/share {collectionID, email, encryptedKey, role}`; `POST /collections/unshare {collectionID, email}` | `sharees[]` (`User.fromMap`) | `collections` | matches: `leave_collection_without_body`, `public_link_management` |
| `shareBatch` | `POST /collections/share/batch {shares[{collectionID, email, encryptedKey, role}]}` | `sharees[]` | same | matches: `bulk_and_batch_sharing` |
| `shareBulk` / `unshareBulk` (library sharing) | `POST /collections/share/bulk {recipientUserID, recipientEmail, source, collections[{collectionID, encryptedKey, role}]}`; `POST /collections/unshare/bulk {recipientUserID, source, collectionIDs}` | `results[]{collectionID, status ∈ shared/already_shared/unshared/already_unshared/not_shared/blocked_previous_removal/failed}` | same | matches: `bulk_and_batch_sharing` |
| `createShareUrl` / `updateShareUrl` | `POST /collections/share-url {collectionID, enableCollect, enableJoin, enableComment}`; `PUT /collections/share-url {collectionID, …props}` | `result` (`PublicURL.fromMap`) | same | matches: `public_link_management` |
| `deleteShareUrl` | `DELETE /collections/share-url/{id}` | status | same | matches: same test |
| public album viewing | `GET /public-collection/info`, `POST /public-collection/verify-password {passHash}` → `jwtToken`, `GET /public-collection/diff?sinceTime`, `GET /public-collection/files/{download,thumbnail}/v3/{id}` (404 → legacy `/files/download|preview/{id}`), `GET /public-collection/files/data/{fetch,preview}` | as museum | `public_collection` | matches (existing `public_collection_links`) |

### Upload

| Caller | Request | App reads | Our route | Status / test |
| --- | --- | --- | --- | --- |
| `validateUploadEligibility` | `GET /files/upload-eligibility` (404 → `GET /files/upload-urls?count=1`) | status | `files` | matches: `upload_then_collection_diff_loop` |
| `getUploadUrl` | `POST /files/upload-url {contentLength, contentMD5}` | `url`, `objectKey` | same | matches: every upload test |
| object PUT (`_s3Dio`) | `PUT url` with `Content-Length`, `Content-MD5`, octet-stream | status | `/storage/*` | matches: same |
| `getMultipartUploadUrl` | `POST /files/multipart-upload-url {contentLength, partLength (20 MiB), partMd5s[]}` | `urls{objectKey, partURLs[], completeURL}` (flat accepted too); count must equal `ceil(contentLength/partLength)` | same | matches: `multipart_upload_like_the_app` |
| part PUT | `PUT partURL` with `Content-MD5`, octet-stream | **`etag` response header** (empty → `ETAG_MISSING`) | `/storage/*` | matches: same test |
| complete | `POST completeURL`, `Content-Type: text/xml`, `<CompleteMultipartUpload><Part><PartNumber>n</PartNumber><ETag>…</ETag></Part>…` with every `"` stripped | status 200 | `/storage/*` | matches: same test |
| `createFile` | `POST /files` into one of the user's own collections (collaborators upload to their own album, then `add-files`; a shared album → 403, as in museum) `{collectionID, encryptedKey, keyDecryptionNonce, file{objectKey, decryptionHeader, size}, thumbnail{…}, metadata{encryptedData, decryptionHeader}, pubMagicMetadata?}` | `id`, `updationTime`, `ownerID` | same | matches: every upload test |
| `updateFile` | `PUT /files/update {id, file, thumbnail, metadata}` | `id`, `updationTime` | same | matches: `file_edits_and_magic_metadata` |
| `FileMagicGateway` | `PUT /files/{magic,public-magic}-metadata {metadataList[{id, magicMetadata}]}` | status; 409 → retry | same | matches: `file_edits_and_magic_metadata` |
| `getFilesInfo` / `getFilesSize` / `getDuplicates` | `POST /files/info {fileIDs}` → `filesInfo[]{id, fileInfo.fileSize}`; `POST /files/size` → `size`; `GET /files/duplicates` → `duplicates[]{fileIDs, size}` | | same | matches: `upload_then_collection_diff_loop`, `file_edits_and_magic_metadata` |

### Download

| Caller | Request | App reads | Our route | Status / test |
| --- | --- | --- | --- | --- |
| `FileUrl.tryGetV3Url` | `GET /files/{download,thumbnail}/v3/{id}` (`X-Auth-Token`) | `url`; 404 → legacy for 1 h | `files` | matches: `download_urls_and_ranges` |
| `DownloadManager._resolveDownloadUrl` (legacy) | `GET /files/download/{id}`, `followRedirects: false`, accepts 300–399 | `Location` | same | matches: same test |
| chunked download | `GET signedURL` with `Range: bytes=a-b` | 206 body | `/storage/*` | matches: same test |
| thumbnails (legacy) | `GET /files/preview/{id}` | body or redirect | same | matches: `recovery_key_passkey_recovery_and_legacy_thumbnail` |

### Trash

| Caller | Request | App reads | Our route | Status / test |
| --- | --- | --- | --- | --- |
| `getDiff` (sync loop) | `GET /trash/v2/diff?sinceTime` | `diff[]{file, updatedAt, isDeleted, isRestored, deleteBy}`, `hasMore` | `trash` | matches: `trash_flow` |
| `trashFiles` | `POST /files/trash {items[{fileID, collectionID}]}` | status | `files` | matches: same |
| `deleteFiles` / `emptyTrash` | `POST /trash/delete {fileIDs}`; `POST /trash/empty {lastUpdatedAt}` | status | `trash` | matches: same (delete: existing `library_flow`) |

### Faces / ML file data, streamable video previews

| Caller | Request | App reads | Our route | Status / test |
| --- | --- | --- | --- | --- |
| `putFileData` | `PUT /files/data {fileID, type:"mldata", encryptedData, decryptionHeader}` | status | `file_data` | matches: `ml_file_data_and_status_diff_loop` |
| `fetchFileData` | `POST /files/data/fetch {fileIDs, type}` | `data[]{fileID, type, encryptedData, decryptionHeader}`, **`pendingIndexFileIDs` and `errFileIDs` cast `as List`**, so they must never be null | same | matches: same test |
| `fetchSingleFileData` | `GET /files/data/fetch?fileID&type&preferNoContent=true` | 200 `data{encryptedData, decryptionHeader}`, 204/404 → none | same | matches: same test |
| `getStatusDiff` (loop until a page is empty) | `POST /files/data/status-diff {lastUpdatedAt}` | `diff[]{fileID, userID, type, size (int, required), updatedAt, isDeleted?, objectID?, objectNonce?}` | same | matches: same test (terminates) |
| `getPreviewUploadUrl` (v1) | `GET /files/data/preview-upload-url?fileID&type=vid_preview` → PUT **without** Content-MD5 | `url`, `objectID` | same | matches: `video_preview_upload_like_the_app` |
| `getPreviewUploadUrlV2` | `POST /files/data/preview-upload-url {fileID, type, contentLength, contentMD5}` → PUT with Content-MD5 | `url`, `objectID` | same | matches: same test |
| `putVideoData` | `PUT /files/video-data {fileID, objectID, objectSize, playlist, playlistHeader}` | status | same | matches: same test |
| `getPreview` | `GET /files/data/preview?fileID&type` | `url` | same | matches: same test |
| public variants | `GET {base}/public-collection/files/data/{fetch,preview}` | same shapes | `public_collection` | matches (existing `public_collection_links`) |

### User entities (people, cgroups, locations, smart albums, memories, library sharing)

| Caller | Request | App reads | Our route | Status / test |
| --- | --- | --- | --- | --- |
| `createKey` / `getKey` | `POST /user-entity/key {type, encryptedKey, header}`; `GET /user-entity/key?type` (404 → create) | `userID?`, `encryptedKey`, `header`, `createdAt?`, `type` | `user_entity` | matches: `user_entity_flow` |
| `createEntity` | `POST /user-entity/entity {encryptedData, id?, header, type}`; custom ids `sa_{uid}_{cid}` (smart_album), `ls_{owner}_{recipient}` (library_share) | `id`, `userID`, `encryptedData`, `header`, `isDeleted`, `createdAt`, `updatedAt` | same | matches: same test |
| `updateEntity` | `PUT /user-entity/entity {id, encryptedData, header, type, expectedUpdatedAt?}` | same; **409 when stale** → merge and retry | same | matches: same test |
| `deleteEntity` | `DELETE /user-entity/entity?id=` | status | same | matches: same test |
| `getDiff` | `GET /user-entity/entity/diff?sinceTime&limit=500&type` | `diff[]` | same | matches: same test |

Types the app uses: `location`, `person`, `cgroup`, `smart_album`, `memory`,
`library_share`, plus `contact` from the contacts core.

### Contacts (Rust `ente_contacts`)

| Caller | Request | App reads | Our route | Status / test |
| --- | --- | --- | --- | --- |
| root key | `GET /user-entity/key?type=contact` (404 → create); `POST /user-entity/key` (**409 → re-read**, same key → 200) | `encryptedKey`, `header` | `user_entity` | matches: `contacts_root_key_conflict_and_diff` |
| CRUD | `POST /contacts {contactUserID, encryptedKey, encryptedData}`, `GET/PUT/DELETE /contacts/{id}` | `id, contactUserID, email?, profilePictureAttachmentID?, encryptedKey?, encryptedData?, isDeleted, createdAt, updatedAt` | `contacts` | matches (existing `contacts_api`) |
| diff | `GET /contacts/diff?sinceTime&limit` | `diff[]` | same | matches: `contacts_root_key_conflict_and_diff` |
| attachments | `POST /attachments/profile_picture/upload-url {contentLength, contentMD5}` → `attachmentID`, `url`; `PUT url` (Content-MD5 only); `PUT /contacts/{id}/attachments/profile_picture {attachmentID, size}`; `GET /attachments/profile_picture/{id}` → `url`; `DELETE /contacts/{id}/attachments/profile_picture` | | same | matches (existing `contacts_api`) |

### Legacy: trusted contacts (Rust `ente_legacy`)

| Caller | Request | App reads | Our route | Status / test |
| --- | --- | --- | --- | --- |
| `info` | `GET /emergency-contacts/info` | `contacts`, `recoverSessions`, `othersEmergencyContact`, `othersRecoverySession`: all **non-null arrays**; records `{user{id,email}, emergencyContact{id,email}, state, recoveryNoticeInDays (int)}`; sessions `{id, user, emergencyContact, status ∈ INITIATED/WAITING/REJECTED/RECOVERED/STOPPED/READY, waitTill, createdAt}` | `emergency` | matches: `legacy_contacts_core_shapes` |
| `add_contact` | `GET /users/public-key?email` (404 → "not on Ente"), then `POST /emergency-contacts/add {email, encryptedKey, recoveryNoticeInDays: int \| null}` | status | same | matches: same test (explicit null accepted) |
| `update_contact` | `POST /emergency-contacts/update {userID, emergencyContactID, state}` | status | same | matches: same test |
| `update_recovery_notice` | `POST /emergency-contacts/update-recovery-notice {emergencyContactID, recoveryNoticeInDays}` | code `ACTIVE_RECOVERY_SESSION` | same | matches: same test |
| start / stop / reject / approve | `POST /emergency-contacts/start-recovery {userID, emergencyContactID}`; `…/{stop,reject,approve}-recovery {id, userID, emergencyContactID}` | status | same | matches: same test (start), existing `legacy_emergency` (others) |
| `recovery_bundle` | `GET /emergency-contacts/recovery-info/{id}` | `encryptedKey`, `userKeyAttr` (KeyAttributes with `masterKeyEncryptedWithRecoveryKey`, `masterKeyDecryptionNonce`) | same | matches (existing `legacy_emergency`) |
| `change_password` | `POST /emergency-contacts/init-change-password {recoveryID, setupSRPRequest{srpUserID, srpSalt, srpVerifier, srpA}}` → `setupID`, `srpB`; `POST /emergency-contacts/change-password {recoveryID, updateSrpAndKeysRequest{setupID, srpM1, updatedKeyAttr{kekSalt, encryptedKey, keyDecryptionNonce, memLimit, opsLimit}}}` → `srpM2` | | same | matches (existing `legacy_emergency` SRP client) |

### Legacy kits (Rust `ente_legacy`, owner side; recovery runs on the web)

| Caller | Request | App reads | Our route | Status / test |
| --- | --- | --- | --- | --- |
| `create_kit` | `POST /legacy-kits {id (uuid), variant: 1, noticePeriodInHours ∈ {0,24,168,360,720}, encryptedRecoveryBlob, authPublicKey, encryptedOwnerBlob}` | `{id, variant (u8), noticePeriodInHours, legacyUrl (String, required), encryptedOwnerBlob, createdAt, updatedAt, activeRecoverySession?}` | `legacy_kit` | matches: `legacy_kits_core_shapes` |
| `kits` | `GET /legacy-kits` | `kits[]` (same record) | same | matches: same test |
| `download_kit_shares` | `GET /legacy-kits/{id}/download-content` | `id`, `variant`, `encryptedOwnerBlob` | same | matches: same test |
| `kit_recovery_session` | `GET /legacy-kits/{id}/recovery-session` | `session?{id, kitID, status, waitTill, createdAt}`, `initiators[]` (**non-null**) | same | matches: same test |
| `update_kit_recovery_notice` | `POST /legacy-kits/update-recovery-notice {kitID, noticePeriodInHours}` | code `ACTIVE_RECOVERY_SESSION` | same | matches: same test |
| `block_kit_recovery` | `POST /legacy-kits/block-recovery {kitID}` | status (404 without a session, as in museum) | same | matches: same test |
| `delete_kit` | `DELETE /legacy-kits/{id}` | status | same | matches: same test |

### Cast

| Caller | Request | App reads | Our route | Status / test |
| --- | --- | --- | --- | --- |
| `getPublicKeys` | `GET /cast/device-info/{code}` | `publicKey`, `pqPublicKey?`; 404 → none, 403 → IP mismatch (museum only logs it, and so do we) | `cast` | matches: `cast_pairing_and_sessions` |
| `publishCastPayload` | `POST /cast/cast-data {deviceCode, encPayload, collectionID, castToken}` | optional `deviceID` | same | matches: same test |
| `getAllCastSessions` | `GET /cast/device-info` | `devices[]{collectionID, deviceIP, deviceID, lastUsedAt (µs), deviceName?}` | same | matches: same test |
| `revokeSessionByID` / `revokeAllTokens` | `DELETE /cast/device-info/{deviceID}`; `DELETE /cast/revoke-all-tokens` | status | same | matches: same test |

### Memories share

| Caller | Request | App reads | Our route | Status / test |
| --- | --- | --- | --- | --- |
| `_createMemoryLink` | `POST /memory-share {type: share\|lane, metadataCipher, metadataNonce, encryptedKey, keyDecryptionNonce, memoryHash, files[{fileID, position, encryptedKey, keyDecryptionNonce}]}` | `memoryShare{id int, type, metadataCipher?, metadataNonce?, encryptedKey, keyDecryptionNonce, accessToken, isDeleted?, createdAt int, updatedAt?, url, memoryHash?, previewUploadedFileID?, fileCount?}` | `memory_share` | matches: `memory_share_like_the_app` |
| list / delete | `GET /memory-share` → `memoryShares[]`; `DELETE /memory-share/{id}` | | same | matches: same test |
| `getPublicMemoryFiles` | `GET /public-memory/files` + `X-Auth-Access-Token` | `files[]{file{id, …}, …}` | same | matches: same test |

### Comments and reactions

| Caller | Request | App reads | Our route | Status / test |
| --- | --- | --- | --- | --- |
| `createComment` | `POST /comments {id: "cmt_"+21, collectionID, cipher, nonce, fileID?, parentCommentID?}` | `id` | `social` | matches: `social_client_ids_and_diffs` |
| `updateComment` / `deleteComment` | `PUT /comments/{id} {cipher, nonce}`; `DELETE /comments/{id}` | status | same | matches: same test |
| `upsertReaction` | `PUT /reactions {id: "rct_"+21, collectionID, cipher (100 bytes padded → 156 b64 chars), nonce, fileID?, commentID?}`; `fileID` repeated for file-scoped comments | `id` | same | matches: same test |
| `deleteReaction` | `DELETE /reactions/{id}` | status | same | matches: same test |
| diffs | `GET /comments/diff`, `/reactions/diff`, `/social/diff?collectionID&commentsSinceTime&reactionsSinceTime&limit&fileID` | `comments[]`, `reactions[]` (`id`, `collectionID`, `userID`, `createdAt`, `updatedAt` required), `hasMore*` | same | matches: same test |
| `fetchLatestUpdates` | `GET /comments-reactions/updated-at` | `updates[]{collectionID, commentsUpdatedAt?, reactionsUpdatedAt?, anonProfilesUpdatedAt?}` | same | matches: same test |
| `getAnonProfiles` | `GET /social/anon-profiles?collectionID` | `profiles` | same | matches: same test |

### Push, billing, storage bonus

| Caller | Request | App reads | Our route | Status / test |
| --- | --- | --- | --- | --- |
| `PushGateway.registerToken` | `POST /push/token {fcmToken, platform, apnsToken: String \| null, notificationKey: String \| null}` | status | `push` | **mismatch**: 400 on `apnsToken: null`. `push_token_with_nulls` (ignored) |
| `getUserPlans` / `getSubscription` | `GET /billing/user-plans` → `plans[]`; `GET /billing/subscription` → `subscription` | | `billing` (stub) | matches: `flags_billing_and_bonus_views` |
| `verifySubscription` | `POST /billing/verify-subscription {paymentProvider, productID, verificationData}` | `subscription`; 409 handled | same | matches (stub, existing) |
| Stripe | `POST /billing/stripe/{cancel,activate}-subscription` (no body), `GET /billing/stripe/customer-portal?redirectURL` → `url` | | same (stub) | matches (stub; not used on self-hosted) |
| storage bonus | `GET /storage-bonus/referral-view`, `GET /storage-bonus/details`, `POST /storage-bonus/referral-claim?code=` (no body), `POST /storage-bonus/change-code?code=` + `{code}` | `ReferralView`, `BonusDetails` | `storage_bonus` | matches: `flags_billing_and_bonus_views`, `referral_claim_code_in_query_without_body`, `storage_bonus_change_code` |
