# Grep audit after the strip (fork-web part 1)

Scope: `web/apps/{photos,accounts,albums,share,cast,embed,memories}`,
`web/packages`, `desktop/{src,build,scripts,package.json,electron-builder.yml}`.
Excluded: `node_modules`, generated `pkg/`, build output.

Patterns: `ente.io`, `ente.com`, `ente` (user-visible, case-insensitive, with
inflections, see "`ente` (user-visible)" below), `duck`, `mascot`,
`referral`, `merch`, `rate us`, `subscription`, `sentry`, `firebase`,
`play.google`, `apps.apple`.

Command used for the host and keyword patterns (case-insensitive):

```sh
grep -rniE "ente\.io|ente\.com|duck|mascot|referral|merch|rate us|subscription|sentry|firebase|play\.google|apps\.apple" \
  web/apps/{photos,accounts,albums,share,cast,embed,memories} web/packages \
  desktop/src desktop/build desktop/scripts desktop/package.json desktop/electron-builder.yml \
  --exclude-dir=node_modules --exclude-dir=pkg --exclude-dir=out --exclude-dir=.next
```

## No hits

`merch`, `rate us`, `sentry`, `firebase`, `play.google`, `apps.apple`: no hits
left in the scope (the Play Store / App Store footer in
`GalleryMedia.tsx` was removed). There is no Sentry/analytics SDK in these
apps; logs stay local ("View logs" saves a file or opens the log folder).

## `ente.io` / `ente.com`: no Ente host is contacted

The Ente CDN, uploader and redirect branches (thumbnails/files/public-albums/
cast-albums.ente.com, photos/albums/ente.com redirects, the
`ente://albums.ente.io` deep link) were removed in the second strip pass, so
media always comes from the configured server. Left, all outside Photovault's
own apps or not network calls:

| File                                                                          | What                                                                        | Why left                                                                                                                                                       |
| ----------------------------------------------------------------------------- | --------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `packages/accounts/components/auth/SignUpForm.tsx` (ente.com/terms, /privacy) | terms checkbox                                                              | rendered only for non-Photovault apps (Auth/Locker), out of scope                                                                                              |
| `packages/accounts/pages/two-factor/recover.tsx` (support@ente.com)           | support e-mail                                                              | rendered only for non-Photovault apps (Auth/Locker); Photovault apps say "contact your server's administrator" (`/photovault/info` has no admin contact field) |
| `apps/photos/src/components/collections/AlbumCastDialog.tsx` (cast.ente.com)  | comment on Ente's Chromecast receiver app id `F5BCEC64`                     | Auto pair is hidden; the id in `utils/chromecast-sender.ts` is unchanged (pending a Photovault receiver registration)                                          |
| `scripts/photovault-vendor-assets.mjs` (assets.ente.com)                      | build-time download of the places/cities/ffmpeg data into `/static-assets/` | build step only, overridable with `PHOTOVAULT_ASSETS_SOURCE`; the apps never contact it                                                                        |
| `rust/crates/ml` (models.ente.com)                                            | ML model downloads (desktop native ML)                                      | pending a decision on where models come from                                                                                                                   |

Not network calls / not shown:

| File                                                                                                                                   | What                                                                                | Why left                                                                                                                                                                         |
| -------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `apps/photos/src/pages/index.tsx` (`.ente.com`/`.ente.io` hostname checks)                                                             | hides the server picker on Ente's own domains                                       | never true on a Photovault host; host comparison only                                                                                                                            |
| `apps/accounts/src/services/passkey.ts` (`.ente.com`/`.ente.io`, `ente:`/`enteauth:`/… schemes)                                        | passkey redirect allow-list                                                         | security logic, left as upstream; Photovault only adds "same origin is allowed"                                                                                                  |
| `apps/accounts/README.md`, `packages/media/collection.ts`, `packages/base/origins.ts`, `apps/albums/.../join-public-album-redirect.ts` | comments                                                                            | not shipped as text                                                                                                                                                              |
| `apps/cast/src/pages/index.tsx`                                                                                                        | `PHOTOVAULT: strip` comment naming what was removed                                 | comment                                                                                                                                                                          |
| `apps/photos/tests/{feed-social-api,quick-links,links-view}.test.ts(x)`                                                                | example URLs in unit tests                                                          | tests only                                                                                                                                                                       |
| `packages/base/locales/*`                                                                                                              | none left except the `title_auth`/`title_locker` values ("Ente Auth"/"Ente Locker") | used only by Ente's Auth and Locker apps (outside this fork's scope); Photovault code no longer reads them. Checked by `scripts/photovault-locale-brand.mjs --check` (see below) |

Outside the photos scope but worth knowing: `web/apps/{auth,locker,ensu,paste,photobook,space,twoof3,legacy,payments}`
still contain Ente links by design (left functionally alone). `web/Dockerfile`
and `web/docs` are upstream documentation/build files, not served by
Photovault.

## `ente` (user-visible)

An earlier version of this audit only looked for a capitalised "Ente" in
string literals and missed lowercase and inflected forms (`WatchFolder.tsx`
"Point ente at a folder", de-DE "des Entes"/"Entes Hilfe", fi-FI
"Enten"/"Entestä"), the new en-US `pv_server_ente_compatible` text, and the
desktop package metadata. All of those are fixed now. Coverage of this pass:

1. Code strings: `node scripts/photovault-brand-audit.mjs` parses every
   `.ts`/`.tsx` file (TypeScript AST, not grep) under `apps/{photos,albums,accounts,cast,embed}/src`
   and `packages` (excluding `node_modules`, `pkg`, `out`, `.next`, `tests`,
   `.d.ts`) and prints every string literal, template literal part and JSX
   text (JSX attribute strings included) matching `\bente(?!r|nd|nt)[\p{L}\d]*`
   case-insensitively; import/export module names are skipped. 606 files, 128
   hits, all listed by kind below.
2. Locale values: `node scripts/photovault-locale-brand.mjs --check` over all
   `packages/base/locales/*/translation.json` values for
   `\bEnte(?!r|nd|nt)\p{Ll}*` or a lowercase `\bente\b`; plus a manual grep
   for transliterations (Энте, エンテ, 엔테, Έντε, إنتي, …): nothing outside
   `title_auth`/`title_locker`.
3. Desktop: `grep -niE "\bente\w*"` over `desktop/package.json`,
   `desktop/electron-builder.yml`, `desktop/build/*.xml`, and quoted strings in
   `desktop/src`.

Not covered: `.js`/`.mjs` config files, CSS files, SVG/PNG content (images are
listed in PHOTOVAULT.md "Images"), and apps outside the shipped set (`auth`,
`locker`, `ensu`, `memories`, `paste`, `photobook`, `space`, `twoof3`,
`legacy`, `payments`, `share`).

Code-string hits (`photovault-brand-audit.mjs`) by kind:

- The one intended mention: `packages/base/components/utils/photovault/encore-tokens.ts`
  `pvCredit = "Based on Ente"` (Settings → Help → About); the desktop About
  panel uses the same plain-text line (`desktop/src/main/photovault/brand.ts`,
  `desktop/src/main.ts`).
- Ente-hosted URLs: only in comments now (see the table above).
- Shown only in Ente's other apps, never in Photovault apps:
  `packages/base/app.ts` titles "Ente Auth", "Ente Locker", "Ente Legacy Kit",
  "Ente Space"; `packages/accounts/components/auth/SignUpForm.tsx`
  (ente.com/terms, /privacy); `packages/accounts/pages/two-factor/recover.tsx`
  (support@ente.com); `apps/accounts/src/pages/passkeys/verify.tsx` "Locker is
  available only to Ente photos paid users…" (only when Ente's Locker app
  started the passkey flow); `packages/accounts/components/RecoveryKey.tsx`
  `ente-recovery-key.txt` (Photovault apps use `photovault-recovery-key.txt`).
- Identifiers, not text: the album filter id `"ente"` (`utils/album-filters.ts`,
  `use-gallery-preferences.ts`, `use-gallery-navigation.ts`,
  `AlbumsOverview.tsx`, `AlbumsSection.tsx`, `use-album-actions.ts`,
  `FeedSection.tsx`; its label is "Photovault"), the server-check kind
  `"ente"` (`photovault/server.ts`, `server-check.ts`; its text is
  "Connected to a compatible server (no Photovault info)"), CSS classes and
  element ids (`pswp-ente*`, `ente-chevron-right`, `ente-pswp-*`,
  `ente-mc-*`, `ente-quality-menu`, `ente-settings-menu`, `ente-progress-*`,
  `ente-logo`, `ente-video`, `ente-vanilla-video`,
  `ente-public-single-file-viewer`, `ente-image-decoded`), storage keys
  (`ente_anon_identity`, `ente_join_album_context`, `ente-files` IndexedDB,
  `ente-app-lock-unlock-attempt`), client package headers `io.ente.*`, the
  `ente:`/`enteauth:`/`entelocker:`/`ente-cli:` URL schemes and `ente://app`,
  asset paths (`/images/ente-circular.png`, now the Photovault icon;
  `ente_empty_state_figma.svg`, now the glyph), and test module names
  (`packages/contacts/index.test.ts`).
- Logs only: `packages/base/log-web.ts` "Starting ente-…" banner and
  `ImageEditorOverlay.tsx` "Error saving copy to ente" (`log.error`); they
  appear only in the log file a user can choose to save.

Desktop package metadata:

- Fixed: `package.json` `description` is "Desktop client for Photovault" (it
  was "… (based on Ente Photos)", shown by deb/rpm/pacman tools);
  `electron-builder.yml` no longer has `azureSignOptions` naming
  "ENTE TECHNOLOGIES, INC." / `EnteTrustCertProfile` / `EnteTechnologiesInc`.
- Left (identifiers, see desktop/PHOTOVAULT.md "Notes"): `package.json`
  `name` `"ente"` — electron-builder also uses it as the Linux package and
  executable name, so `apt`/`dnf`/`pacman` list the package as `ente` (with
  description "Desktop client for Photovault"). Renaming it would change the
  install path and is left to the desktop owner. Also `appId`
  `io.ente.bhari-frame`, the AppStream id `io.ente.photos` and launchable
  `ente.desktop`, the `ente://` scheme, the auto-launch entry `ente`, the log
  file `ente.log`, temp folder names.

## `duck` / `mascot`

All remaining hits are file names or style names; the artwork itself is now
the Photovault glyph (see PHOTOVAULT.md, "Images"):

- `apps/{photos,albums}/public/images/upload-complete-duck.svg`,
  `upload-progress-sleeping-duck.svg`, `apps/photos/public/images/empty-album/duck.svg`,
  `auth-ducky.svg`: glyph SVGs; the comment inside says what they replace.
  File names kept so upstream code paths stay identical.
- `packages/gallery/components/UploadCompletion.tsx`,
  `upload-progress/UploadProgressDialog.tsx` (`mascotSx`),
  `apps/photos/src/components/{SearchBar.tsx,gallery/sections/albums/AlbumEmptyState.tsx}`,
  `packages/accounts/components/auth/AuthPageShell.tsx`: references to those
  file names, a style constant and a code comment.

## `referral`

- `packages/accounts/{components/SignUpContents.tsx,components/auth/SignUpForm.tsx,pages/verify.tsx,services/accounts-db.ts}`:
  Ente's "How did you hear about us?" sign-up field. Hidden in Photovault apps,
  so nothing is stashed and nothing is sent (the plumbing stays for Auth/Locker).
- `apps/albums/.../public-collection.ts`, `public-albums-fdb.ts`,
  `apps/embed/src/services/public-collection.ts`: the server's public-collection
  response carries a `referralCode`; it is stored but never shown.
- `apps/photos/src/services/search/sidebar-search-registry.ts`,
  `apps/photos/src/components/sidebar/Sidebar.tsx`: `PHOTOVAULT: strip` comments
  and a no-op `showReferrals` passed to the (unchanged) action dispatcher.
- `packages/new/photos/services/search/types.ts`: the `utility.referrals`
  action id stays in the union type (no menu entry or search entry uses it).
- Locales: `referral_*` keys other than the removed `referral_share_text` are
  unused now; kept to keep locale merges trivial.

## `subscription`

Real usage data still flows through Ente's types, which are named
"subscription"; no plan names, prices, upgrade or payment UI remain:

- `packages/new/photos/services/user-details.ts`: the `Subscription` schema of
  `/users/details/v2` (storage limit = `subscription.storage`, per the
  storage-usage contract) and helpers like `isPartOfFamily`. Payment functions
  were removed.
- `apps/photos/src/components/sidebar/account/SubscriptionCard.tsx`: the storage
  card ("X of Y used", photo count, family split). Shows real numbers only.
- `apps/photos/src/components/sidebar/{Sidebar.tsx,accounts/Account.tsx,accounts/subscription.ts,accounts/ManageMemberSubscription.tsx}`:
  component and function names (`openManageSubscription`,
  `ManageMemberSubscription` = family member's "leave family" sheet); they now
  lead to family management.
- `packages/gallery/services/upload/upload-service.ts`,
  `apps/photos/src/components/Upload.tsx`, `apps/albums/.../Upload.tsx`: the
  server's "Subscription expired" upload error is still recognised and shown as
  a plain notification (no renew/upgrade action).
- `apps/photos/src/components/gallery/hooks/{use-gallery-operations,use-gallery-uploads}.ts`:
  the `onShowSubscription` prop chain. `Upload.tsx` no longer calls it; the
  "subscription" settings action it maps to now opens family management.
- `apps/photos/src/components/gallery/media/use-file-mutations.ts`:
  `share_link_subscription_required` error text from the server (HTTP 402).
- `apps/photos/src/components/sidebar/theme.ts`: CSS selector
  `[data-sidebar-subscription]` for the storage card.
- `packages/new/photos/services/search/types.ts`, `apps/photos/tests/*`:
  action ids and test fixtures.
- Locales: plan/subscription strings are unused now; kept to keep locale merges
  trivial.

## Third-party (not Ente) hosts still contacted

For completeness, these are not Ente-run and belong to features that are kept:

- OpenStreetMap tiles and Nominatim search (map, location editing).
- Stadia Maps tiles and reverse geocoding (trip-layout public albums).
- Google Cast SDK from gstatic.com (cast).
