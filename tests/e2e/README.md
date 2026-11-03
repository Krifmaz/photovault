# End-to-end tests with the real Ente apps

These tests drive the **unmodified** upstream Ente Photos web app (and,
optionally, the Electron desktop app) against a photovault server, with
Playwright. Upstream is pinned in [`UPSTREAM_COMMIT`](UPSTREAM_COMMIT)
(ente-io/ente main, 2026-11-03).

```sh
tests/e2e/run.sh                      # everything
tests/e2e/run.sh signup upload_photos_and_video open_photo   # a subset (deps are not auto-run)
```

`run.sh` builds `photovault` (debug), starts it with a temporary SQLite
database, temporary local object storage and hard-coded OTTs
(`[hardcoded_ott]`: `e2e@example.com`, `e2e2@example.com`, `e2e3@example.com`
→ `123456`), serves the built Photos app on `:13000` and the public Albums app
on `:13002` with a tiny static server (`lib/static-server.mjs`, nginx-style
`$uri`, `$uri.html`, `/index.html` fallback), and runs `runner.mjs`.
Results: `artifacts/results.json`; each failing test gets
`artifacts/<test>/` with a screenshot and the visible text and controls of
every open page, the error, console errors and failed API calls.
`artifacts/server.log` is the server's log. `artifacts/` is gitignored.

Ports are fixed (API 18080, web 13000, albums 13002) because the API origin is
baked into the web build.

## Requirements

Node 22, Playwright with Chromium (`require('playwright')`), ffmpeg (test
media is generated at run time: two JPEGs and a 2 s H.264 MP4), Rust with the
`wasm32-unknown-unknown` target, git, curl. No Docker.

## The web build (`build-web.sh`)

Sparse-clones `web/` and `rust/` of ente-io/ente at the pinned commit into
`$PHOTOVAULT_E2E_CACHE` (default `~/.cache/photovault-e2e`), then runs
upstream's own commands: `npm ci`, `npm run build:photos`,
`npm run build:albums` (upstream switched from yarn to npm workspaces; the
apps compile parts of Ente's Rust code to WebAssembly with wasm-pack). The
API origin is set with the official self-hosting variable
`NEXT_PUBLIC_ENTE_ENDPOINT=http://localhost:18080`; no source is changed.
Output is cached as `web-<commit>-<endpoint>/{photos,albums}` and reused.

Timing on this 4-core machine (shared with other builds): cold build about
13-14 min (`npm ci` + compiling wasm-bindgen-cli about 8 min, Next.js builds
of Photos and Albums about 5.5 min); cached: instant. The cache takes about
2.7 GB (checkout + node_modules + wasm target) plus 28 MB per build.

wasm-pack downloads `wasm-opt` (binaryen) with its own HTTP client, which
ignores the system CA store and so fails behind TLS-intercepting proxies; the
script therefore fetches binaryen `version_117` with curl into the cache and
puts it on `PATH`, which wasm-pack uses instead.

## Test cases

| Test | What it does in the UI |
| --- | --- |
| `web_app_loads` | landing page shows the custom API host |
| `dev_settings_custom_endpoint` | 7 taps on the landing page → Developer settings → set server endpoint (app validates `GET /ping` → `{"message":"pong"}`) |
| `signup` | email + password + terms → OTT (6-digit form) → recovery key screen (checks the 24 words and the saved file match) → plan page → empty gallery |
| `logout_login` | Settings → Logout → email → password (SRP) → gallery |
| `upload_photos_and_video` | Upload → Files → 2 JPEGs + 1 MP4 → new album → Start upload → "3 uploaded" |
| `gallery_after_reload` | reload; collections and file diff fetched again from the server |
| `open_photo` | viewer decrypts the full-size original (≥ 640 px wide) |
| `play_video` | viewer loads the MP4 (320 px video) |
| `download_matches_original` | viewer → More → Download; SHA-256 equals the uploaded file |
| `create_album` | sidebar New album |
| `add_and_remove_from_album` | select → Add → album; then Remove from the album |
| `favorite` | viewer Favorite → Favorites album shows it |
| `hide_and_unhide` | select → Hide → Hidden (re-auth) → Unhide to album |
| `trash_restore_delete` | Delete → Trash → Restore → Delete → Empty trash |
| `search_by_name_and_date` | search "e2e-photo" (File name suggestion) and the current month (Date suggestion) |
| `signup_second_user` | second account in a separate browser context |
| `share_album_with_user` | Share album → Add viewers → second user sees the album and its files |
| `public_link` | Create public link → open it in a fresh logged-out context (Albums app) → decrypted thumbnails |
| `legacy_trusted_contact` | **skipped**: the Photos web app at this commit has no Legacy UI (only mobile and the Locker web app); see the mobile checklist |
| `active_sessions` | Settings → Active sessions (re-auth) lists the current session |
| `change_password` | Settings → Change password (re-auth) → logout → login with the new password |
| `delete_account` | second user: Settings → Delete account → reason → summary → confirm; email login no longer offers a password |

Tests share two accounts. A test whose dependency failed is reported as
`blocked` (not run). Selectors use the en-US UI strings and ARIA labels.

### Validated against museum

The harness was validated against Ente's own server (museum, built from the
same commit) with Postgres and an S3 mock (moto) using
[`reference/museum.yaml`](reference/museum.yaml) and
`PHOTOVAULT_E2E_EXTERNAL_API=1` (or by running `runner.mjs` directly):
**21 passed, 1 skipped** of 22 (2026-11-03, full run about 3 min). Every failure against photovault is
therefore a server incompatibility, not a harness problem.

### Status against photovault

2026-11-03, all workstreams merged: **21 passed, 1 skipped** of 22, the same
as museum (`run.sh` about 4 min with a cached web build). `desktop-smoke.sh`
passes against photovault too.

| Test | museum | photovault |
| --- | --- | --- |
| `web_app_loads` … `active_sessions`, `delete_account` (19 tests) | pass | pass |
| `public_link` | pass | pass (link points at the albums app via `[accounts] public_albums_url`) |
| `change_password` | pass | pass (needed `setupId` accepted in `POST /users/srp/update`) |
| `legacy_trusted_contact` | skipped | skipped |

### Server requirements the tests surfaced

* `GET /ping` must return `{"message":"pong"}` (museum also sends `"id"`); the
  web Developer settings, desktop and mobile apps use it to accept a custom
  server.
* On `localhost` (and api.ente.com) the client rejects a login whose sealed
  token is not `b"auth-token" || 32 random bytes` (see
  `server/tests/vectors/README.md`).
* Public links: the server returns the full link URL; for the test it must use
  the albums app origin `http://localhost:13002` (museum: `apps.public-albums`,
  photovault: `[accounts] public_albums_url`, which `run.sh` sets). The test
  falls back to the local albums app with the same token if the link points
  elsewhere, and notes it.
* JSON field names are matched case-insensitively by museum (Go): the apps'
  Rust client sends `setupId` (not `setupID`) to `POST /users/srp/update`.
* Object up/downloads happen from the browser at `http://localhost:13000`, so
  signed storage URLs need CORS (PUT, GET, ETag exposed).

## Desktop (Electron)

The desktop app bundles the same web code (`desktop/` builds `web/apps/photos`
with `_ENTE_IS_DESKTOP=1` and serves it at `ente://app`). Release builds have
no build-time API override, so users point it at a server exactly like the
web app: on the login screen, tap/click the empty background **7 times** →
**Developer settings** → *Server endpoint* `https://photos.example.com` →
Save. The app checks `GET <endpoint>/ping` and stores the origin (key
`apiOrigin` in its local KV store); the endpoint is shown under the login form.
To go back, open the dialog again and clear the field.

`desktop-smoke.sh` builds the unmodified desktop app (`npm ci`,
`npm run build:quick` → `electron-builder --dir`, cached in
`$PHOTOVAULT_E2E_CACHE/desktop-<commit>`), launches it headless under
`xvfb-run` with Playwright's Electron driver, sets the endpoint through
Developer settings and checks that login requests go to the server. It needs
a server already listening (`E2E_API_URL`, default `http://localhost:18080`)
and fails early if `GET /ping` does not answer. Upstream's `postinstall` (vips, ffmpeg, Electron native deps,
ONNX Runtime, and a release build of the app's napi Rust addon) is run
explicitly after `npm ci`, so the build also works where npm is configured
with `ignore-scripts=true` (as on this machine).

Validated 2026-11-03 against museum and against photovault: `PASS desktop
smoke: app accepted http://localhost:18080 and called
.../users/srp/attributes?...`. Cold build
about 11 min on this machine (most of it the napi addon and Electron/vips/
ffmpeg/ONNX downloads), reusing the web checkout; cached afterwards.

## Mobile (Flutter) manual checklist

The mobile apps cannot run here. To test by hand on a phone or emulator with
an official build (Play Store / F-Droid / App Store / GitHub APK):

1. Run photovault reachable from the phone (e.g. `https://photos.example.com`
   or `http://<lan-ip>:8080`). Check `curl <url>/ping` returns `{"message":"pong"}`.
2. On the first (onboarding) screen, **tap the screen 7 times** → *Developer
   settings* → enter the server endpoint → Save. The app pings it; the
   endpoint is shown on the onboarding screen.
3. Sign up with a new email; get the OTT from the server log (or
   `[hardcoded_ott]`); set a password; save the recovery key.
4. Grant photo access, pick folders to back up; confirm uploads complete and
   the server's storage grows.
5. Log out and in again (password); also try "Forgot password" with the
   recovery key.
6. Open photos and play a video; zoom; check thumbnails in the grid.
7. Create an album, add and remove photos, rename, delete it.
8. Favourite, hide (biometric/PIN re-auth), archive, trash, restore, empty trash.
9. Share an album with a second account (viewer and collaborator); the second
   device sees it and, as collaborator, can add photos.
10. Create a public link (with password and expiry); open it in a browser.
11. **Legacy / trusted contacts** (mobile only): Settings → Account → Legacy →
    add the second account as trusted contact; on the second device accept
    the invite; start recovery and check the wait-period notice; cancel it.
12. Search by file name, date, album and location (photos with GPS EXIF).
13. Settings → Security → Active sessions: terminate another device's session.
14. Change password; check the other device is logged out.
15. Two-factor (TOTP) setup and login; passkeys if the server supports them.
16. Delete account; confirm the email can sign up afresh.
17. Ente Auth app (same server): add a TOTP code, check it syncs to a second device.
