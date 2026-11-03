# Photovault patches to Ente's web apps

This folder is Ente's `web/` (ente-io/ente) with a small, theme-only patch set
for Photovault. Every changed hunk carries a `PHOTOVAULT` comment where the file
format allows comments. Photovault is based on Ente (ente.com).

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
  preview description. (The `ente.com` preview image URLs are only used for
  Ente's own production endpoint and are left as is.)
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
  instead of green.
- `apps/memories/public/images/ente-brand-tag.svg`: Ente tag replaced by the
  Photovault glyph (same size).

Credit:

- `apps/photos/src/components/sidebar/Help.tsx`: "Photovault — Based on Ente
  (ente.com)" row in Settings → Help.

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

Text on the (now cream) accent:

- `apps/photos/src/components/gallery/navigation/PhotosNavigationShell.tsx`,
  `FixCreationTime.tsx`, `gallery/sections/cleanup/LargeFilesView.tsx`: white
  text/tick on `accent.main` now uses `accent.contrastText`.

Strings:

- `packages/base/locales/*/translation.json`: the brand name "Ente" / "Ente
  Photos" in values replaced by "Photovault" (scripted; keys, URLs, e-mail
  addresses and "Ente Auth/Locker/Legacy" untouched). These files are shared
  with Ente's other web apps.

## Merging upstream

Conflicts will mostly be in the files above. Re-apply by keeping upstream's
change and re-adding the `PHOTOVAULT` hunk; for locales, take upstream's file
and re-run the brand-name replacement (only `\bEnte\b` not followed by
Auth/Locker/Legacy/Ensu, and "Ente Photos").
