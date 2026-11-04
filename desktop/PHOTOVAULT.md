# Photovault patches to the Ente desktop app

This folder is a light fork of Ente's Electron app (`ente-io/ente` `desktop/`).
In-app colours, fonts and strings come from the web app (`web/`), which the
desktop app loads; this fork only rebrands the Electron shell. Every code hunk
is marked with a `PHOTOVAULT` comment.

**Theme tokens:** `src/main/photovault/brand.ts` (app name, credit line, and the
window background / title-bar colours, mirroring `branding/encore-tokens.json`).

## Changed files

- `src/main/photovault/brand.ts`: new; brand name, plain-text "Based on Ente" credit, Encore bg/text colours.
- `src/main.ts`: window background and title bar overlay use Encore tokens; tray tooltip; About panel with name and the plain-text credit (no website link).
- `src/main/ipc.ts`: title bar overlay colours on theme change use Encore tokens.
- `src/main/menu.ts`: "Ente"/"Ente Photos" menu and tray labels become "Photovault". Strip: the Help menu (it only opened ente.com/help) and "Check for Updates..." are removed.
- `src/main/services/app-update.ts`: strip: the auto-updater never runs. electron-updater would poll Ente's GitHub release feed and offer Ente's app as an update; Photovault releases are installed manually.
- `package.json`: `productName` "Photovault" (app/window name, installer and artifact names) and `description` "Desktop client for Photovault" (shown as the deb/rpm/pacman package description; no Ente mention); strip: `repository` points at this repository and `author` is "Photovault developers" (no Ente e-mail). `build-renderer` and `build-renderer:ci` run `web/scripts/photovault-vendor-assets.mjs` (Node, so it also works on Windows) before `npm run build:photos`, so the bundled web app has its `/static-assets/` (places, city search; upstream fetched these from Ente's CDN at runtime). A failed download fails the build; `PHOTOVAULT_ASSETS_SOURCE=<url or dir>` picks a mirror and `PHOTOVAULT_SKIP_ASSETS=1` builds without them. JSON has no comments, so this is listed here only.
- `electron-builder.yml`: URL protocol display name; the `ente://` scheme and `appId` are unchanged. Strip: explicit Linux `maintainer` (package.json's author has no e-mail any more); Windows `azureSignOptions` removed (they named Ente's signing account and publisher), so Windows builds are unsigned until Photovault has its own certificate.
- `build/io.ente.photos.appdata.xml`: Linux AppStream name, developer, description and brand colours. Strip: no Ente homepage/bug tracker/release URLs, no Ente-hosted screenshot, no credit (the credit lives in About).
- `build/icon.png`, `build/icon.icns`, `build/icon.ico`, `build/window-icon.png`: rendered from `branding/icon.svg`.
- `build/taskbar-icon*.png`: tray icons from `branding/glyph.svg`, monochrome white; `build/taskbar-icon-Template*.png`: the same in black for macOS template images.

## Notes

- Changing `productName` also changes Electron's `userData` folder
  (e.g. `~/.config/Photovault` instead of `~/.config/ente`), so Photovault
  does not share local state with an installed Ente app.
- Kept unchanged on purpose (identifiers, not display names): `appId`,
  package `name`, the `ente://` scheme, the auto-launch entry name `ente`,
  log file `ente.log`, temp folder names and the client package header.
