# Grep audit after the strip (fork-web part 1)

Scope: `web/apps/{photos,accounts,albums,share,cast,embed,memories}`,
`web/packages`, `desktop/{src,build,scripts,package.json,electron-builder.yml}`.
Excluded: `node_modules`, generated `pkg/`, build output.

Patterns: `ente.io`, `ente.com`, `ente` (user-visible), `duck`, `mascot`,
`referral`, `merch`, `rate us`, `subscription`, `sentry`, `firebase`,
`play.google`, `apps.apple`.

Command used (case-insensitive):

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

## `ente.io` / `ente.com`: all unreachable, non-network, or not user-visible

Unreachable: `isCustomAPIOrigin` is always `true` and `customAPIOrigin()`
always returns a server (see `packages/base/origins.ts`), so these Ente-hosted
branches never run. They are left as in upstream to keep merges small.

| File                                                                                                 | What                                   | Why left                                                                                                                                                                                                                                       |
| ---------------------------------------------------------------------------------------------------- | -------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `packages/gallery/services/download.ts` (thumbnails/files/public-albums.ente.com)                    | CDN fetches used when no custom origin | unreachable                                                                                                                                                                                                                                    |
| `packages/media/public-albums.ts` (`public-albums.ente.com`)                                         | last fallback after `customOrigin`     | unreachable (`customOrigin` is always set)                                                                                                                                                                                                     |
| `apps/albums/.../thumbnail-manager.ts`, `.../download-manager.ts` (public-albums.ente.com)           | same CDN branch                        | unreachable                                                                                                                                                                                                                                    |
| `apps/cast/src/services/render.ts` (cast-albums.ente.com)                                            | same CDN branch                        | unreachable                                                                                                                                                                                                                                    |
| `apps/albums/.../PublicAlbumPage.tsx` (photos.ente.com, albums.ente.com, ente.com)                   | redirects for Ente-hosted builds       | guarded by `!isCustomAPIOrigin` → unreachable                                                                                                                                                                                                  |
| `apps/embed/src/pages/index.tsx` (ente.com)                                                          | redirect for Ente-hosted builds        | guarded by `!isCustomAPIOrigin` → unreachable                                                                                                                                                                                                  |
| `apps/albums/.../join-public-album-redirect.ts` (photos.ente.com, `ente://albums.ente.io` deep link) | "Join album" from a public link        | Ente disables Join for custom origins (`joinEnabled` needs `!isCustomAPIOrigin` outside dev builds), so unreachable. Re-enabling it for Photovault (redirect to our web app, `photovault://` deep link) belongs to the server-connection work. |
| `packages/accounts/components/auth/SignUpForm.tsx` (ente.com/terms, /privacy)                        | terms checkbox                         | rendered only for non-Photovault apps (Auth/Locker)                                                                                                                                                                                            |
| `packages/accounts/pages/two-factor/recover.tsx` (support@ente.com)                                  | support e-mail                         | rendered only for non-Photovault apps                                                                                                                                                                                                          |

Not network calls / not shown:

| File                                                                                                                                   | What                                                                                | Why left                                                                                                   |
| -------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------- |
| `apps/photos/src/pages/index.tsx` (`.ente.com`/`.ente.io` hostname checks)                                                             | hides the server picker on Ente's own domains                                       | never true on a Photovault host; host comparison only                                                      |
| `apps/accounts/src/services/passkey.ts` (`.ente.com`/`.ente.io`)                                                                       | passkey redirect allow-list used only when the page itself is on an Ente domain     | security logic, never true on a Photovault host; left untouched on purpose                                 |
| `apps/accounts/README.md`, `packages/media/collection.ts`, `packages/base/origins.ts`, `apps/albums/.../join-public-album-redirect.ts` | comments                                                                            | not shipped as text                                                                                        |
| `apps/cast/src/pages/index.tsx`                                                                                                        | `PHOTOVAULT: strip` comment naming what was removed                                 | comment                                                                                                    |
| `apps/photos/tests/{feed-social-api,quick-links,links-view}.test.ts(x)`                                                                | example URLs in unit tests                                                          | tests only                                                                                                 |
| `packages/base/locales/*`                                                                                                              | none left except the `title_auth`/`title_locker` values ("Ente Auth"/"Ente Locker") | used only by Ente's Auth and Locker apps (outside this fork's scope); Photovault code no longer reads them |

Outside the photos scope but worth knowing: `web/apps/{auth,locker,ensu,paste,photobook,space,twoof3,legacy,payments}`
still contain Ente links by design (left functionally alone). `web/Dockerfile`
and `web/docs` are upstream documentation/build files, not served by
Photovault.

## `ente` (user-visible)

Checked string literals and JSX text in scope for the word "Ente":

- `packages/base/components/utils/photovault/encore-tokens.ts`: `pvCredit =
"Based on Ente"`. This is the one intended mention (Settings → Help → About).
- `desktop/src/main/photovault/brand.ts`: `upstreamCredit = "Based on Ente"`,
  the desktop About panel credit (same line, plain text, no link).
- `packages/base/app.ts`: static titles "Ente Auth", "Ente Locker", "Ente
  Legacy Kit", "Ente Space" for Ente's other apps (not Photovault apps).
- `apps/accounts/src/pages/passkeys/verify.tsx`: "Locker is available only to
  Ente photos paid users…", shown only when the passkey flow was started by
  Ente's Locker app. Not reachable from Photovault.
- `packages/base/components/ente-wordmark.ts` / `EnteLogo.tsx`: Ente's
  wordmark paths, rendered only for non-Photovault apps (`isPhotovaultApp`).
- Identifiers that are not text: package names (`ente-base`, …), CSS classes
  (`pswp-ente`), `EnteFile`, the `ente://` URL scheme, `io.ente.*` client
  package headers and desktop `appId`, log file `ente.log`, the
  `ente-web-logs-*` name was changed to `photovault-web-logs-*`.

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
