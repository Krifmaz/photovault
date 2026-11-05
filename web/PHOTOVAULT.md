# Photovault patches to Ente's web apps

This folder is Ente's `web/` (ente-io/ente) with a small patch set for
Photovault: a retheme (`PHOTOVAULT: theme` / `branding`) and a strip of Ente's
company features, links and phone-home calls (`PHOTOVAULT: strip`). Every
changed hunk carries a `PHOTOVAULT` comment where the file format allows
comments. Photovault is based on Ente; the only user-visible credit is the
plain-text "Based on Ente" in Settings → Help → About.

## Where the theme lives

- `packages/base/components/utils/photovault/encore-tokens.ts`: the Encore
  colours, radii and fonts, mirroring `../branding/encore-tokens.json` (change
  that file first, then this one). Also the brand name, the credit line, and
  `isPhotovaultApp()` (photos, accounts, albums, cast, embed, photobook).
- `packages/base/components/utils/photovault/encore-theme.ts`: maps the tokens
  onto the palette shape of Ente's `theme.ts` (light and dark), and the Encore
  component overrides (14 px cards, 10 px controls, cream-to-gold gradient on
  accent buttons).
- Everything else reads these modules or the MUI CSS variables
  (`--mui-palette-accent-main` etc.) derived from them.

Apps not listed in `isPhotovaultApp` (Auth, Locker, Ensu, Paste, Space,
Legacy, Share, Payments, Twoof3) keep Ente's palette and logo. They do pick up
the shared locale strings (see below).

## Changed files

Theme core:

- `packages/base/components/utils/theme.ts`: Photovault apps take colours,
  font stack and component overrides from the Encore modules.
- `packages/base/components/utils/photovault/*` (new): tokens and theme mapping.
- `apps/photos/src/components/sidebar/theme.ts`: the settings sidebar's local
  palette (Ente greys and green) now comes from the tokens.
- `apps/memories/src/pages/_app.tsx`: uses `photosTheme` (Encore) instead of
  `shareTheme`; description says Photovault.

Sign-in pages (shared `ente-accounts` auth shell, used by Photos, Auth and Locker):

- `packages/accounts/components/auth/styles.ts`: optional `photovault` per-scheme
  variable overrides in `AuthPresentationConfig` (unused by Auth/Locker).
- `packages/accounts/components/auth/AuthPageShell.tsx`: page, card and brand
  panel colours read optional variables (fallback = upstream values); the
  wordmark box follows the logo's aspect ratio.
- `packages/accounts/components/auth/Button.tsx`: optional on-primary colour
  and gradient variables for the primary button.
- `apps/photos/src/components/PhotosAuthShell.tsx`: Encore colours for the
  sign-in pages, from the tokens.

Logo, name, icons:

- `packages/base/components/photovault/wordmark.ts`, `PhotovaultLogo.tsx`
  (new): Photovault wordmark (glyph from `branding/glyph.svg` plus name).
- `packages/base/components/EnteLogo.tsx`: renders the Photovault wordmark in
  the Photovault apps.
- `apps/albums/src/public-album/viewer/lib/photoswipe.ts`: Photovault wordmark in
  the public-album viewer.
- `packages/gallery/components/viewer/photoswipe-core.ts`: logo button title.
- `packages/base/app.ts`: static app titles (Photovault).
- `packages/base/components/Head.tsx`: default description, albums title and
  preview description.
- `apps/embed/src/pages/_app.tsx`, `apps/cast/src/pages/index.tsx`: name.
- `apps/photos/src/components/CollectionSelector.tsx`,
  `apps/photos/src/utils/album-filters.ts`: "Ente" album filter label.
- `apps/memories/src/components/{MemoryViewer,LaneMemoryViewer}.tsx`: alt text.
- Images, rendered from `branding/icon-1024.png`: `apps/{photos,albums,accounts,cast,embed,memories,photobook}/public/images/favicon.png`,
  `apps/memories/public/favicon.ico`, `apps/accounts/public/images/ente-circular.png`,
  `apps/photos/public/images/app-icons/{,2.0x/,3.0x/}photos.png`.
- `apps/photos/public/images/auth-ducky.svg`: sign-in illustration replaced by
  the Photovault glyph (same canvas size).
- `apps/photos/public/images/auth-frame-curves.svg`: decorative curves in gold
  instead of green (fallback file). The sign-in pages draw the curves from the
  tokens instead: `apps/photos/src/components/photovault/auth-curves.ts` (new)
  builds the same path as a data URL in a token colour, passed as the
  `curves` (page, per-scheme `accent2`) and `brand-curves` (dark panel,
  `pvDark.accent2`) variables; `AuthPageShell.tsx` reads them with the file as
  fallback. The duck's camera-flash sparkle (`IllustrationClip::after`) is
  turned off with `sparkle-display: none` (the variable is unset in Auth and
  Locker, which keep it).
- `packages/base/components/photovault/wordmark.ts`: the glyph's gradient reads
  `--pv-glyph-from`/`--pv-glyph-to`, which `encore-theme.ts` sets on `:root`
  (CssBaseline) to the scheme's accent: gold to charcoal in light mode (the
  cream glyph was nearly invisible there), gold to cream in dark mode. Always
  dark surfaces pin the dark colours with `pvGlyphDarkVars`: the sign-in
  brand panel (`AuthPageShell.tsx` `Wordmark`), the public-album photo viewer
  (`photoswipe.ts`, `PublicAlbumSingleFileViewer.tsx`) and the trip cover
  (`TripCover.tsx`). Apps without the Encore theme keep the dark colours.
- `apps/memories/public/images/ente-brand-tag.svg`: Ente tag replaced by the
  Photovault glyph (same size).

Credit:

- `apps/photos/src/components/sidebar/Help.tsx`: About block in Settings → Help
  with the name, version (desktop) and the plain-text `pvCredit` "Based on Ente"
  (no link, no logo). `pvCreditURL` was removed from `encore-tokens.ts`.

Hard-coded Ente greens (`#08c225`, `#1db954`, `rgba(8, 194, 37, …)`) pointed at
the tokens (`pvGold`, `pvGoldHover`, `pvOnGold`, `pvAlpha`), with dark text on
gold where the text was white:

- `apps/photos/src/components/{UploadConfirmationDialog,FamilyManagement}.tsx`,
  `app-lock/AppLockUnlockForm.tsx`, `gallery/sections/albums/GalleryEmptyState.tsx`,
  `sidebar/account/SubscriptionCard.tsx`, `collections/{EditAlbumDetailsDialog,CoverPhotoGrid}.tsx`
- `apps/albums/src/public-album/page/PublicAlbumPage.tsx`,
  `upload/components/UploaderNameInput.tsx`, `viewer/components/{AddNameModal,CommentsSidebar,LikesSidebar,PublicAlbumSingleFileViewer,PublicCommentModal,PublicFeedSidebar,PublicLikeModal}.tsx`
- `apps/memories/src/components/{LaneMemoryViewer,PublicMemoryMedia,PublicMemoryPageShell}.tsx`,
  `PublicMemoryViewerShared.ts`
- `apps/embed/src/components/EmbedFileListWithViewer.tsx`
- `packages/gallery/components/{UploadCompletion.tsx,upload-progress-stats.ts}`,
  `upload-progress/UploadProgressDetails.tsx`, `upload/CanvasReadbackBlockedDialog.tsx`,
  `viewer/{CommentsSidebar,LikeAlbumSelectorModal,LikesSidebar}.tsx`
- `packages/base/components/SingleInput.styles.ts`
- SVGs: `apps/photos/src/components/app-lock/lock.svg`,
  `apps/photos/public/images/empty-state/no-matches.svg`,
  `apps/photos/public/images/empty-album/background.svg`,
  `apps/albums/src/public-album/upload/components/enter-name.svg`

Primary buttons (token rule: `accent` to `accent2` gradient with `onAccent`
text, per scheme, via `pvVar`):

- `packages/base/components/SingleInput.styles.ts` (`v2SubmitButtonSx`, used by
  the Family screen's "Add family member" and other dialogs), and the
  `color="accent"` buttons in `apps/photos/src/components/{UploadConfirmationDialog,collections/EditAlbumDetailsDialog,app-lock/AppLockUnlockForm,gallery/sections/albums/GalleryEmptyState}.tsx`.
  These had fixed dark text (`pvOnGold`) on the theme's light-mode charcoal
  gradient; they now use `onAccent` (cream in light mode, dark in dark mode).

Text on the (now cream) accent:

- `apps/photos/src/components/gallery/navigation/PhotosNavigationShell.tsx`,
  `FixCreationTime.tsx`, `gallery/sections/cleanup/LargeFilesView.tsx`: white
  text/tick on `accent.main` now uses `accent.contrastText`.

Strings:

- `packages/base/locales/*/translation.json`: the brand name "Ente" / "Ente
  Photos" in values replaced by "Photovault" (scripted; keys, URLs, e-mail
  addresses and "Ente Auth/Locker/Legacy" untouched). These files are shared
  with Ente's other web apps.

## Strip (company features, Ente links, phone-home)

Kept exactly as upstream: backup, albums, timeline, search, ML, memories, map,
sharing and public links, collect links, hidden albums, trash, free up space,
Legacy, family, streamable videos, cast, export, app lock, 2FA and passkeys.

Server and network:

- `packages/base/origins.ts`: `customAPIOrigin()` always resolves (saved
  server → `NEXT_PUBLIC_ENTE_ENDPOINT` → the page's own http(s) origin →
  `http://localhost:8080`) and `isCustomAPIOrigin` is always true, so Ente's
  existing self-hosted code paths run everywhere. The uploader worker and Ente
  og:image metadata are gone; the Ente CDN and redirect branches are removed
  too (next item). Also new: `staticAssetURL()` for data
  Ente loads from assets.ente.com. Desktop builds vendor the same files
  (`desktop/package.json` `build-renderer*`, see desktop/PHOTOVAULT.md).
- No Ente host fallback at all (files, thumbnails and public/cast album media
  always come from the configured server, Ente's self-hosted path):
  `packages/gallery/services/download.ts` (thumbnails/files/public-albums.ente.com
  branches; the `background` download option only chose Ente's proxy, so the
  transport ignores it), `packages/media/public-albums.ts`
  (`public-albums.ente.com` fallback), `apps/albums/.../thumbnail-manager.ts`
  and `.../download-manager.ts` (public-albums.ente.com),
  `apps/cast/src/services/render.ts` (cast-albums.ente.com),
  `apps/photobook/src/lib/ente/import.ts` (public-albums.ente.com default).
- `apps/albums/.../PublicAlbumPage.tsx`: "Login" on an expired link goes to
  Photos at the root of this origin (was `router.push("/")`, i.e. the albums
  app itself, or photos.ente.com); no trip-album redirect to albums.ente.com
  (the check always returns false) and no ente.com redirect without a token.
  `apps/embed/src/pages/index.tsx`: no ente.com redirect without a token.
- `apps/photos/src/services/places.ts`, `packages/new/photos/services/search/worker.ts`,
  `packages/gallery/services/ffmpeg/web.ts`: places index, world cities and the
  ffmpeg.wasm core load from `/static-assets/` on our origin.
  `scripts/photovault-vendor-assets.mjs` (new; `.sh` is a wrapper) copies them into
  `apps/photos/public/static-assets/` (git-ignored) before a build. It also
  copies `location/v2/{countries,disputes}.bin`, which only the mobile app
  uses (it loads its location index from `<server>/static-assets/location/v2`).
- `packages/new/photos/services/settings.ts`: default cast/embed URLs and the
  custom-domain CNAME point at this server instead of cast/embed/my.ente.com.
- `packages/new/photos/services/user-details.ts`: plans, Stripe and payments-app
  functions removed (`payments.ente.com`).
- `apps/{photos,albums,embed}/public/_headers`: no CSP reports to
  csp-reporter.ente.com and no Ente hosts in the (report-only) policy.
- `apps/{photos,albums}/public/.well-known/{assetlinks.json,apple-app-site-association}`:
  emptied (they granted Ente's Android/iOS apps credential sharing and link
  handling for the domain). JSON, so no comment; listed here. Add Photovault's
  app ids and signing fingerprints here when they exist.

Photos app (settings and gallery):

- `components/sidebar/Sidebar.tsx`: no Referrals row/screen, plan selector or
  subscription status line. Where Ente opened the plan selector (storage card,
  "subscription" action) we open family management, so family stays reachable.
- `components/sidebar/accounts/{Account,subscription,ManageMemberSubscription}.tsx`,
  `components/FamilyManagement.tsx`: "Manage plan" row becomes "Family" with
  storage only (upstream's family/member-limit storage, no billing period); no
  Stripe customer portal; families can be created without a paid plan (the
  server decides). No "plan" wording on the family screens: "Family" / "Share
  your storage with up to five family members…", "Shared storage", "Leave
  family", "You are in a family managed by", and on
  `apps/accounts/src/pages/family.tsx` "…invited you to share their storage"
  (titles use Ente's translated `family` key). The reworded strings are new
  `pv_family_*` / `pv_leave_family*` keys in `en-US` only, read through
  `packages/base/photovault/i18n.ts` (new, `pvT(pvKey, upstreamKey)`): a
  locale without the `pv_*` key shows Ente's translated string (e.g.
  `leave_family_plan`) rather than English. Known limitation: in those
  locales the family screens still say "plan" until the `pv_*` strings are
  translated.
- Deleted: `components/PlanSelector.tsx`, `pages/plan.tsx`,
  `components/sidebar/ReferralSettings.tsx`, `services/storage-bonus.ts`,
  `components/{WhatsNew,DesktopWhatsNew}.tsx`, `services/changelog.ts`,
  `components/gallery/hooks/use-subscription-completion-redirect.tsx`.
- `components/gallery/hooks/use-gallery-library.ts`, `auth-page-config.ts`: no
  plan page after sign-up (the "just signed up" flag is cleared as before).
- `pages/gallery.tsx`: no "What's new" dialog.
- `components/Upload.tsx`: quota/expiry notifications without "Upgrade" /
  "Renew" (payments) actions.
- `components/utils/download.tsx`: "desktop only" dialogs without download
  links to ente.com.
- `components/sidebar/Help.tsx`: Ente help centre, blog, feature requests and
  support e-mail removed; View logs kept; About added (see Credit).
- `components/sidebar/preferences/{Preferences,MLSettings}.tsx`,
  `components/{FixCreationTime,TakeoutOptions}.tsx`,
  `components/auth/DevSettingsDialog.tsx`: Ente help/privacy links removed;
  "Faster upload" (Ente's Cloudflare upload proxy) toggle removed, and the
  Advanced section is desktop-only (it only holds desktop options now).
- `services/search/sidebar-search-registry.ts`: search entries for the removed
  rows dropped.
- `components/gallery/media/GalleryMedia.tsx`: no "install our mobile app"
  Play Store / App Store footer.
- `components/sidebar/{EnteAppIcon.tsx,account/DeleteAccount.tsx}`: Ente
  Auth/Locker logos become plain icons, labels "Auth"/"Locker"; the logo PNGs
  `public/images/app-icons/**/{auth,locker}.png` are deleted.
- `components/collections/CollectionShare.tsx`: no embed.ente.com fallback.
- `components/collections/AlbumCastDialog.tsx`: no Chromecast "Auto pair"; it
  launches Ente's registered receiver app (`F5BCEC64`, cast.ente.com talking
  to Ente's API), which cannot pair with a Photovault server. The dialog opens
  on the PIN flow (`utils/chromecast-sender.ts` itself is unchanged).
- `components/sidebar/WatchFolder.tsx`: empty state says "Point Photovault at
  a folder…" (was "Point ente…").
- `services/export.ts`: export folder "Photovault" instead of "Ente Photos".

Shared packages and other apps:

- `packages/base/components/Head.tsx`: no Ente-hosted og:image/og:video
  metadata, no "based on Ente" meta description. Deleted the matching Ente
  preview assets (`apps/{photos,albums,share}/public/images/preview.png`,
  `apps/albums/public/videos/preview.mp4`, `apps/memories/public/images/memories-meta*.png`).
- `packages/base/app.ts`: share app title "Photovault".
- `packages/accounts/components/{SignUpContents.tsx,auth/SignUpForm.tsx}`: in
  Photovault apps, no "How did you hear about us?" field and no Ente
  terms/privacy checkbox (treated as accepted). Auth/Locker unchanged.
- `packages/accounts/pages/two-factor/recover.tsx`: Photovault apps say to
  contact the server's administrator instead of support@ente.com.
- `packages/accounts/components/RecoveryKey.tsx`: recovery key file name
  `photovault-recovery-key.txt` in Photovault apps.
- `packages/gallery/components/upload/{DefaultOptions,CanvasReadbackBlockedDialog}.tsx`,
  `packages/new/photos/components/DevSettings.tsx`: Ente help links removed.
- `packages/gallery/components/viewer/photoswipe-core.ts`: the public-album
  wordmark is no longer a link to ente.com.
- `packages/contacts/legacy/components/LegacyAddContactContent.tsx`: Legacy
  texts without "Ente".
- `apps/albums/.../access/services/join-public-album-redirect.ts`: "Join
  album" goes to Photos at the root of this origin (`/?joinAlbum=…`), not
  photos.ente.com. The `ente://albums.ente.io` Android deep link code is
  removed (Ente's own app also handles it and would send the token to Ente's
  servers).
- `apps/albums`: no "Join Ente" buttons (`page/PublicAlbumPage.tsx`,
  `components/TripLayout/{TopNavButtons,MobileNavBar}.tsx`,
  `viewer/components/PublicAlbumSingleFileViewer.tsx`), wordmark not linked;
  `access/utils/external-links.ts` (ente.com/get) deleted.
- `apps/embed/src/components/EmbedFileListWithViewer.tsx`: "powered by" wordmark
  not linked.
- `apps/memories`: no og:image, no "Join Ente"/"Get Ente Photos" buttons, brand
  tag not linked, no redirect to ente.com for a missing token.
- `apps/cast/src/pages/index.tsx`: no "Visit ente.com/cast for help".
- `apps/share`: our wordmark (unlinked) instead of the Ente Locker logo (logo
  SVGs deleted); no redirect to ente.com/locker; title "Photovault".
- `apps/accounts/src/pages/{index,family,recover-account}.tsx`: no redirect to
  ente.com; "Open Photovault" goes to this server's web app (`/`).

Images (no comments possible in PNGs; SVGs carry one): duck mascot art replaced
by the Photovault glyph at the same canvas size, keeping file names so code
paths are unchanged: `apps/{photos,albums}/public/images/upload-{complete,progress-sleeping}-duck.svg`,
`apps/photos/public/images/empty-album/duck.svg`,
`apps/photos/public/images/empty-state/{ente_empty_state_figma,no-matches}.svg`,
`apps/photos/public/images/feed/empty.png`,
`apps/photos/public/images/family-plan/1x.png` (the Family screen's 3D figures
illustration),
`apps/accounts/public/images/family-invite{,-accepted,-invalid}.png` (rendered
from `branding/icon-1024.png`).

Locales (`packages/base/locales/*/translation.json`, scripted): removed the
unused `referral_share_text` and `delete_account_multi_app_warning` keys
everywhere and `auth_developer_settings_subtitle` outside en-US (it named
api.ente.io; other languages fall back to English); replaced the brand word in
forms the first pass missed (lowercase "ente", "Enteä", Ente attached to CJK,
Hangul or Telugu text), and inflected forms (de-DE `password_generation_failed`
"des Entes" → "von Photovault", `ente_help` "Entes Hilfe" → "Photovault-Hilfe";
fi-FI `password_generation_failed` "Enten" → "Photovaultin",
`referral_source_hint` "Entestä" → "Photovaultista"). `title_auth`/`title_locker`
("Ente Auth"/"Ente Locker") stay: only Ente's Auth and Locker apps use them;
no other locale value contains an Ente word (checked by the script below).
en-US `view_logs_message` no longer says "email to us".

`scripts/photovault-locale-brand.mjs` (new) is the replacement as a script:
generic rules ("Ente Photos", `Ente` not followed by a Latin letter or by
Auth/Locker/Legacy/Ensu) plus per-locale inflections, then a check that lists
any value still containing `\bEnte` + lowercase letters (other than
Enter/Entend/Entent) or a lowercase `ente`; `--check` only checks.
`scripts/photovault-brand-audit.mjs` (new) lists every string literal,
template part and JSX text in the shipped apps that contains an Ente word
(case-insensitive, inflections included); PHOTOVAULT-GREP.md explains each
remaining hit.

See `PHOTOVAULT-GREP.md` for the remaining grep hits and why each is left.
Repo-wide, `scripts/strip-report.py` (with `scripts/strip-allowlist.toml`)
checks the web, mobile and desktop sources and the static export in CI and
writes `docs/src/strip-report.md`; every remaining hit needs an entry there.

## Server connection (`PHOTOVAULT: connect`)

The server contract is coded in one file:
`packages/base/photovault/server.ts` (the `GET /photovault/info` model from
`contracts/server-info.md`, address normalisation, the default-server order,
the QR payload, and the info check with Ente's `/ping` as fallback). Change
that file when the contract changes. Unit tests:
`apps/photos/tests/photovault-server.test.ts`.

Which server the apps talk to (`customAPIOrigin()` in `packages/base/origins.ts`):

1. the server chosen on this device (login page), kept in localStorage under
   `photovaultServer` (survives logout) and mirrored to Ente's KV
   `apiOrigin` so workers see it;
2. `NEXT_PUBLIC_ENTE_ENDPOINT` at build time (Ente's variable; empty = unset);
3. the origin the page was served from (the Photovault server serves the web
   app and the API on one port, so this is the default for the server image);
4. `http://localhost:8080` (only reachable for a desktop app never pointed at
   a server).

API paths are Ente's, unchanged.

Assumption (documented, not asked): public albums, cast and accounts pages are
served by the same server under `/albums`, `/cast`, `/accounts`
(`contracts/apps.json`). Public album links themselves are built by the
server (`albumsUrl`), as in Ente. The cast and embed URLs the Photos app uses
default to `<server>/cast` and `<server>/embed`; the server's feature flags
(`castUrl`, `embedUrl`) still override them, as in Ente. Ente's
`NEXT_PUBLIC_ENTE_*` variables keep their upstream meaning.

Changed files:

- `packages/base/photovault/server.ts` (new): the contract model (see above).
- `packages/base/origins.ts`: default order above; `saveServerOrigin()`.
- `packages/base/local-storage.ts`: logout keeps `photovaultServer`.
- `packages/base/env.ts`: `buildEnvBasePath` (the sub-app's Next `basePath`).
- `packages/new/photos/services/settings.ts`: cast/embed defaults follow the API
  server rather than the page origin.
- `apps/photos/src/pages/index.tsx`: Ente's hidden "tap seven times" developer
  endpoint dialog is removed and `components/auth/DevSettingsDialog.tsx`
  deleted; `ServerField` is shown under the login/sign-up form, and the forms'
  own host line is left empty (the field shows it). The `/login` and `/signup`
  routes do the same: `packages/accounts/components/auth/AuthPageProvider.ts`
  has an optional `ServerField` slot (set in `apps/photos/src/auth-page-config.ts`)
  that `packages/accounts/pages/{login,signup}.tsx` render instead of the bare
  host line. In the field's edit state, Connect and Cancel ignore the auth
  button's 2 px hover lift so they stay level.
- `apps/photos/src/components/photovault/{ServerField.tsx,server-check.ts}`
  (new): the collapsed server field (shows name and address; "Change" expands
  it; checks `GET /photovault/info` and shows name + version, or errors such as
  "Can't reach 192.168.1.10:8080. Are you on your home Wi-Fi?" for home-network
  addresses).
- `apps/photos/src/components/sidebar/photovault/ServerSettings.tsx` (new) and
  `sidebar/Sidebar.tsx`: Settings → Server with name, version, home and public
  addresses and a QR code of
  `photovault://connect?url=<publicUrl>&home=<homeUrl>&name=<name>` (a stock
  Ente server gets a QR of its bare address). Not added to settings search.
- `apps/photos/package.json`, `package-lock.json`: `qr@0.6.0` (pure JS, no
  dependencies; already in the lock file for Ente's Legacy and Twoof3 apps)
  added to Photos. JSON, so no comment.
- `packages/base/locales/en-US/translation.json`: `pv_server*` strings (English
  only; other languages fall back to English).
- `apps/photos/src/components/PhotosAuthShell.tsx` (`PHOTOVAULT: strip`): the
  sign-in brand panel's "10 GB free forever" (Ente's plan) and "Independently
  audited" (Ente's audit) bullets are replaced by `pv_auth_own_server` and
  `pv_auth_your_keys` ("No ads, no spying" stays).
- `apps/photos/tests/sidebar-album-search.test.tsx`: expects the album group
  label "Photovault" (renamed by the theme commit; the test still said "Ente").
- `apps/{albums,cast,accounts}/next.config.js`: Next `basePath` `/albums`,
  `/cast`, `/accounts` by default. `PHOTOVAULT_{ALBUMS,CAST,ACCOUNTS}_BASE_PATH=""`
  builds an app for its own host (Ente's layout).
- `apps/albums/src/public-album/page/PublicAlbumPage.tsx`: a path token
  (`/albums/<token>`) ignores the base path. Links with `?t=` are unaffected.
  "Join album" is shown whenever the link has `enableJoin` (Ente hid it for
  self-hosted servers only because Photos lived on another origin).
- `apps/embed/next.config.js`, `apps/embed/src/pages/index.tsx`: the embed app
  (the public link "Embed" option) gets `basePath` `/embed`
  (`PHOTOVAULT_EMBED_BASE_PATH=""` for its own host) and a path token ignores
  it. `apps/photos/.../CollectionShare.tsx` builds the iframe URL as
  `<embedURL>/?t=<token>#<key>` instead of swapping the link's origin (which
  kept the `/albums` path). `scripts/photovault-build.sh` builds it into
  `<web dir>/embed/`.
- `packages/base/origins.ts`: the public-link pages (albums, embed) and the
  cast receiver use the build-time endpoint, else the origin that served them,
  and ignore a server saved by a Photos user on the same origin (public links
  belong to the server that serves them; avoids mixed content and "invalid
  link" against a different saved server).
- `apps/accounts/src/services/passkey.ts`: a same-origin passkey redirect
  (Photos at `/` next to Accounts at `/accounts`) is always allowed. Upstream
  restricts redirects when the host ends with `localhost`, so a production
  server at `http://localhost:8080` rejected `/passkeys/finish`.
- `apps/photos/src/components/collections/AlbumCastDialog.tsx`: the "Visit …
  on the device" step shows host and path (`photos.example.com/cast`), not
  only the host, since the receiver lives under `/cast`.
- Sign-up: `packages/base/photovault/server.ts` requires `product:
"photovault"` and `signupsAllowed` (a 200 without them is "invalid").
  `ServerField` reports each check (`onCheck`); `pages/index.tsx` hides "Sign
  up" and returns to the login view when the server says
  `signupsAllowed: false`, by not passing `onSignUp` to
  `packages/accounts/components/LoginContents.tsx` (now optional; `auth/LoginForm.tsx`
  hides the prompt without it). A stock Ente server keeps Ente's behaviour.
- `server-check.ts`: an http:// server typed on a page served over https
  (blocked as mixed content) gets "This page is served over HTTPS and can't
  connect to an http:// address…" instead of "Are you on your home Wi-Fi?".
  A stock Ente server shows "Connected to a compatible server (no Photovault
  info)" (the kind stays `ente` internally). This deliberately differs from
  the contract's suggested "Ente-compatible server" wording, because the only
  user-visible mention of Ente is "Based on Ente" in About.

Not done for the web app: the contract's home/public address switching rule
(the web app is served by the server it talks to, so there is nothing to
switch), and showing `/photovault/usage` "unlimited" on the storage card.

## Email code on servers without SMTP (`PHOTOVAULT: email-skip`)

`/photovault/info` may carry `emailVerification`: `"smtp"`, `"log"` or
`"skip"` (contracts/server-info.md). With `"skip"` the server accepts any
6-digit code while an OTT from `POST /users/ott` is pending, so the apps do
not show the "Check your inbox" screen:

- `packages/base/photovault/server.ts`: optional `emailVerification` in the
  info model (a string, so a future value does not make the body invalid),
  `skippedEmailCode` (`"000000"`), `shouldAutoSubmitEmailCode()` (only
  `"skip"`, and not for accounts with email 2FA, which still need their real
  code) and `fetchEmailVerificationMode()` (one `GET /photovault/info`, 3 s
  timeout, never throws; a stock Ente server gives no mode). Tests in
  `apps/photos/tests/photovault-server.test.ts`.
- `packages/accounts/services/photovault/email-code.ts` (new): reads the mode
  of the configured server and keeps a one-shot sessionStorage note
  (`photovaultSkipEmailCode` = the email) for /verify.
- `packages/accounts/components/SignUpContents.tsx` and `LoginContents.tsx`
  (the email-code login branch only, not SRP): fetch the mode in parallel
  with `sendOTT` (no extra wait; sign-up awaits it after key generation) and
  leave the note when the code can be skipped.
- `packages/accounts/pages/verify.tsx`: when the note matches the email, shows
  a loading indicator and submits `skippedEmailCode` through the page's own
  `onSubmit`, so success continues exactly as a typed code (`/generate` for a
  new account, `/credentials`, two-factor or passkey). A wrong code or any
  error shows the normal code screen. A reload of /verify shows the code
  screen (the note is consumed).

Absent field, `"smtp"`, `"log"` and stock Ente servers keep upstream's flow.
Keys, SRP and 2FA are unchanged; only the email check is automated.

## Serving the web apps (static export)

Build: `web/scripts/photovault-build.sh` (npm ci, wasm for prelogin/photos/cast,
the `/static-assets/` data files, then `next build` of each app with the
repo's own workspace commands; see the script header for its variables). CI:
`.github/workflows/photovault-web.yml` runs the same and uploads the served
layout as the `photovault-web` artifact, then runs
`scripts/strip-report.py --check --web-out` over it (report uploaded as
`strip-report-web`).

| Build output             | Serve at     | Notes                                                                                                                                                                            |
| ------------------------ | ------------ | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `web/apps/photos/out/`   | `/`          | Photos (login, gallery, settings). Also holds `/static-assets/` and the few `/fonts`, `/images` files the sub-apps reference by absolute path (the build script copies them in). |
| `web/apps/albums/out/`   | `/albums/`   | Public albums and collect links (`basePath` `/albums`). Links: `<publicUrl>/albums/?t=<token>#<key>`, or `/albums/<token>#<key>`.                                                |
| `web/apps/cast/out/`     | `/cast/`     | Cast receiver (pairing code, slideshow).                                                                                                                                         |
| `web/apps/accounts/out/` | `/accounts/` | Passkeys, family invites, account recovery.                                                                                                                                      |
| `web/apps/embed/out/`    | `/embed/`    | Public link "Embed" iframe (`basePath` `/embed`). Not yet copied by the server Dockerfile (see below).                                                                           |

Server rules: try `$uri`, `$uri.html`, `$uri/index.html`; for an extension-less
path that matches nothing, serve the nearest sub-app's `index.html`
(`/albums/…` → `albums/index.html`, likewise `/cast/`, `/accounts/`, `/embed/`, else the
root `index.html`). `_next/static/` is content-hashed and can be cached
immutable. This matches `PHOTOVAULT_WEB_DIR` in `contracts/server-env.md` and
the server branch's Dockerfile (which copies `web/apps/<app>/out`).
`PHOTOVAULT_WEB_OUT=<dir> scripts/photovault-build.sh` also assembles that
layout into one directory.

Watch out: Ente's cast API also lives under `/cast/` (`POST /cast/device-info`,
`GET /cast/cast-data/<code>`, `/cast/diff`, `/cast/files/...` and friends).
The server must route those API paths before the web-dir fallback, and only
fall back to `cast/index.html` for paths that are not API routes.

`web/apps/embed/out/` is served at `/embed/` (public link "Embed" iframe,
`basePath` `/embed`, links `<publicUrl>/embed/?t=<token>#<key>`).
`contracts/apps.json` and the server Dockerfile (which copies only `albums
cast accounts`) do not list it yet; until they do, the server image has no
`/embed/` and the Embed option's iframe shows nothing. The page must also be
frameable from other sites (no `X-Frame-Options: DENY` / restrictive
`frame-ancestors` on `/embed/`). The `memories` app is not built.

## Merging upstream

Conflicts will mostly be in the files above. Re-apply by keeping upstream's
change and re-adding the `PHOTOVAULT` hunk; for locales, take upstream's file
and re-run `node scripts/photovault-locale-brand.mjs` (add any inflected form
its check reports to the per-locale table), then
`node scripts/photovault-brand-audit.mjs` for code strings.

- `packages/accounts/components/auth/LoginForm.tsx`: when the server refuses sign-ups, show "Sign-up is closed on this server. Ask its admin for an account." (`pv_signups_closed`) instead of hiding the prompt silently.
- `packages/accounts/pages/signup.tsx`: a direct /signup visit on a server with sign-ups off goes to /login (which says sign-up is closed).
- `packages/accounts/components/SignUpContents.tsx`: when sign-up fails and the server reports `signupsAllowed: false`, show "Sign-up is closed on this server" instead of "Something went wrong".
- `packages/base/photovault/clipboard.ts` (new), `packages/base/components/utils/hooks.ts`, and the copy call sites in photos/albums: copy also works on plain http:// servers (execCommand fallback, then a prompt to copy by hand).
