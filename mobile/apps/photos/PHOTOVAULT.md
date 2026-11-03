# Photovault patches: Photos app (Flutter)

This app is a light fork of Ente Photos (`ente-io/ente`, `mobile/apps/photos`).
Photovault changes are theme and branding only: no layout, flow, API, crypto or
identifier changes. Every Dart/XML/plist hunk carries a `PHOTOVAULT` comment
(ARB files, PNGs and generated SVGs cannot hold comments and are listed here).

Photovault is based on Ente (ente.com). The credit line is shown at the bottom
of Settings.

## Where the theme lives

- `mobile/packages/ente_components/lib/theme/photovault/encore_tokens.dart`:
  the only place colour, radius and font values are defined. It mirrors
  `branding/encore-tokens.json`; change the JSON first, then this file.
- `mobile/packages/ente_components/lib/theme/photovault/encore_color_tokens.dart`:
  maps the tokens onto the component theme (`ColorTokens`) for
  `ComponentApp.photos` only, with the role mapping documented at the top.
- Everything else (the legacy `EnteColorScheme`, `ThemeData`, the few
  hard-coded colours) references those constants.

Role mapping in short: Ente's brand green (`primary*`, `green*`, `greenBase`)
becomes the Encore accent (cream/gold on charcoal in dark, charcoal/gold in
light) with `onAccent` text; Ente's red `warning*` becomes Encore `err`;
`caution*` becomes Encore amber `warn`; backgrounds, panels, borders and text
use Encore's `bg/bg2/panel/panel2/border/text/muted/faint`. Light mode is kept.
Inter was already bundled, so no font files were added.

## Regenerating icons and logos

`scripts/photovault/render_branding.js` renders `branding/icon.svg` and
`branding/glyph.svg` into every Android/iOS icon, splash and in-app logo, and
`scripts/photovault/wordmark.py` outlines the "Photovault" wordmark. See the
header of `render_branding.js` for usage. Run it again after an upstream merge
that adds or resizes icon assets.

## Files changed

Theme tokens (new):
- `../../packages/ente_components/lib/theme/photovault/encore_tokens.dart`: Encore tokens (single source in Flutter).
- `../../packages/ente_components/lib/theme/photovault/encore_color_tokens.dart`: component colour tokens for Photos built from the tokens.

Shared packages (also used by Ente Auth/Locker, see notes below):
- `../../packages/ente_components/lib/theme/colors.dart`: `ColorTokens.forApp(photos)` returns the Encore tokens; new `onPrimary` token (default white, so Auth/Locker are unchanged).
- `../../packages/ente_components/lib/theme/radii.dart`: `Radii.lg` and `Radii.button` use the Encore card radius (14; were 16 and 20).
- `../../packages/ente_components/lib/components/buttons/button_component.dart`: primary button text uses `onPrimary`.
- `../../packages/ente_components/lib/components/buttons/fab_component.dart`: primary FAB foreground uses `onPrimary`.
- `../../packages/ente_components/lib/components/buttons/icon_button_component.dart`: "green" icon button foreground uses `onPrimary`.
- `../../packages/ente_components/lib/components/tag_chip_component.dart`: selected chip text uses `onPrimary`.
- `../../packages/ente_components/lib/components/selection_controls/checkbox_component.dart`: check mark uses `onPrimary`.
- `../../packages/ente_components/lib/components/selection_controls/toggle_switch_component.dart`: Material switch active thumb uses `onPrimary`.
- `../../packages/ente_components/lib/components/avatar_component.dart`: initials on the accent-coloured avatar use `onPrimary`.
- `../../packages/ui/lib/theme/colors.dart`: ente_ui's Photos palette uses the Encore gold and charcoal-to-gold button gradient.
- `../../packages/strings/lib/l10n/arb/strings_*.arb`: brand name "Ente"/"Ente Photos" -> "Photovault" in message values (not keys, URLs, e-mails or "Ente Auth"/"Ente Locker"); English also fixes "an" -> "a".

Photos app theme:
- `lib/theme/colors.dart`: legacy `EnteColorScheme` constants point at the Encore tokens; new `onAccent`.
- `lib/ente_theme_data.dart`: Material `ThemeData` (buttons, switches, checkbox, inputs, cards, dialogs, popup menus, text colours, radii) and colour-scheme extension getters use the tokens.
- `lib/ui/components/models/button_type.dart`: legacy primary button uses the accent with readable text.
- `lib/ui/components/models/button_type_v2.dart`: primary button text uses `onAccent`.
- `lib/ui/account/onboarding_page_scaffold.dart`: onboarding header backdrop is charcoal instead of green.
- `lib/ui/settings_page.dart`: offline login arrow uses `onPrimary`; "More from Ente" footer replaced by the Photovault footer.
- `lib/ui/viewer/people/save_or_edit_person.dart`: badge icon uses `onPrimary`.
- `lib/ui/tools/editor/image_editor/image_editor_text_bar.dart`: the "Green" text swatch stays green (Encore `ok`).
- Hard-coded Ente greens routed through tokens: `lib/ui/account/two_factor_setup_page.dart`, `lib/ui/home/status_bar_widget.dart`, `lib/ui/map/map_gallery_tile_badge.dart`, `lib/ui/rituals/ritual_camera_page.dart`, `lib/ui/rituals/ritual_day_thumbnail.dart`, `lib/ui/rituals/ritual_page.dart`, `lib/ui/settings/backup/backup_item_card.dart`, `lib/ui/settings/storage_card_widget.dart`, `lib/ui/sharing/public_link_enabled_actions_widget.dart`, `lib/ui/social/comment_likes_bottom_sheet.dart`, `lib/ui/social/like_collection_selector_sheet.dart`, `lib/ui/social/likes_bottom_sheet.dart`, `lib/ui/social/widgets/comment_actions_popup.dart`, `lib/ui/social/widgets/comment_like_count_capsule.dart`, `lib/ui/social/widgets/feed_item_widget.dart`, `lib/ui/social/widgets/file_social_overlay.dart`, `lib/ui/viewer/file/video_download_progress_indicator.dart`, `lib/ui/viewer/people/memory_lane_banner.dart`, `lib/ui/viewer/people/person_gallery_suggestion.dart`, `lib/ui/wrapped/cards/badge_card_content.dart` (import in `lib/ui/wrapped/wrapped_viewer_page.dart`).

Photos app branding:
- `lib/ui/photovault/photovault_settings_footer.dart` (new): settings footer with "Based on Ente (ente.com)" and the app version.
- `lib/app.dart`: `MaterialApp.title` is "Photovault".
- `lib/services/notification_service.dart`: default notification channel name is "Photovault" (channel id unchanged).
- `lib/ui/home/grant_permissions_widget.dart`: Photovault wordmark instead of Ente's (wider logo box).
- `lib/ui/wrapped/wrapped_viewer_page.dart`: "Ente Rewind" title -> "Photovault Rewind".
- `assets/ente-branding.svg`, `assets/ente_io_green_black.svg`, `assets/ente_io_green.png`: Photovault wordmark (file names kept so code is untouched).
- `assets/qr_logo.png`, `assets/launcher_icon/*.png`: Photovault icon / glyph.
- `scripts/photovault/render_branding.js`, `scripts/photovault/wordmark.py` (new): asset generator.

Android:
- `android/app/src/main/res/values/strings.xml`, `src/debug/.../strings.xml`, `src/dev/.../strings.xml`: app label "Photovault" (package id unchanged).
- `android/app/src/main/res/values/colors.xml`: adaptive-icon background #1c1c1e.
- `android/app/src/main/res/values-v31/styles.xml`, `values-night-v31/styles.xml`: Android 12 splash background (Encore light/dark bg).
- `android/app/src/main/res/mipmap-*/{ic_launcher,launcher_icon,icon_*}.png`, `src/dev/res/mipmap-*/launcher_icon.png`: legacy launcher icons.
- `android/app/src/main/res/drawable-*/ic_*launcher_foreground.png`: adaptive-icon foregrounds (glyph) and monochrome layer.
- `android/app/src/main/res/drawable-*/splash.png`, `drawable*/android12splash.png`, `drawable*/background.png`: splash images and backgrounds.
- `android/app/src/main/res/drawable/notification_icon.png`: notification glyph.
- `android/app/src/main/play/listings/en-US/graphics/icon/icon.png`: store icon.

iOS:
- `ios/Runner.xcodeproj/project.pbxproj`: `APP_DISPLAY_NAME` and extension `CFBundleDisplayName` -> Photovault (bundle ids unchanged).
- `ios/Runner/Info.plist`: brand name in permission prompts.
- `ios/Runner/Assets.xcassets/*.appiconset/*.png`: app icons (any/dark/tinted) for every icon set.
- `ios/Runner/Assets.xcassets/LaunchImage.imageset/*.png`, `LaunchBackground.imageset/*.png`: launch screen.

## Known gaps

- Ente's other apps are still named where the text is about them ("Ente Auth",
  "Ente Locker", the change-log "Meet Ente Locker" button) and in the upstream
  change log (`lib/ui/notification/update/change_log_strings.dart`); share
  texts still link to ente.com (URLs are not changed by policy).
- Illustrations (ducky artwork, Rive animations, home-screen widget previews,
  store screenshots) still contain Ente's mascot or green.
- The alternate app icons (Settings > App icon) all show the Photovault icon.
