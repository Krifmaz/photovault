# Photovault patches to the Ente desktop app

This folder is a light fork of Ente's Electron app (`ente-io/ente` `desktop/`).
In-app colours, fonts and strings come from the web app (`web/`), which the
desktop app loads; this fork only rebrands the Electron shell. Every code hunk
is marked with a `PHOTOVAULT` comment.

**Theme tokens:** `src/main/photovault/brand.ts` (app name, credit line, and the
window background / title-bar colours, mirroring `branding/encore-tokens.json`).

## Changed files

- `src/main/photovault/brand.ts`: new; brand name, "Based on Ente (ente.com)" credit, Encore bg/text colours.
- `src/main.ts`: window background and title bar overlay use Encore tokens; tray tooltip; About panel with name and credit.
- `src/main/ipc.ts`: title bar overlay colours on theme change use Encore tokens.
- `src/main/menu.ts`: "Ente"/"Ente Photos" menu and tray labels become "Photovault" (Help still opens Ente's help docs).
- `package.json`: `productName` "Photovault" (app/window name, installer and artifact names) and description. JSON has no comments, so this is listed here only.
- `electron-builder.yml`: URL protocol display name; the `ente://` scheme and `appId` are unchanged.
- `build/io.ente.photos.appdata.xml`: Linux AppStream name, developer, description (with credit) and brand colours.
- `build/icon.png`, `build/icon.icns`, `build/icon.ico`, `build/window-icon.png`: rendered from `branding/icon.svg`.
- `build/taskbar-icon*.png`: tray icons from `branding/glyph.svg`, monochrome white; `build/taskbar-icon-Template*.png`: the same in black for macOS template images.

## Notes

- Changing `productName` also changes Electron's `userData` folder
  (e.g. `~/.config/Photovault` instead of `~/.config/ente`), so Photovault
  does not share local state with an installed Ente app.
- Kept unchanged on purpose (identifiers, not display names): `appId`,
  package `name`, the `ente://` scheme, the auto-launch entry name `ente`,
  log file `ente.log`, temp folder names and the client package header.
