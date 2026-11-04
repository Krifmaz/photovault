# Photovault patches: Photos app (Flutter)

This app is a light fork of Ente Photos (`ente-io/ente`, `mobile/apps/photos`).
Photovault changes are: theme and branding, removal of Ente company material
(promos, plans/payments, Ente links and support, mascot, telemetry, Google
services), and server-connection features. Photo features, flows, the API and
all crypto are unchanged. Every Dart/XML/plist/gradle hunk carries a
`PHOTOVAULT` comment (ARB files, PNGs and generated SVGs cannot hold comments
and are listed here).

The only user-visible mention of Ente is the plain-text line "Based on Ente"
in Settings > About (`EncoreTokens.credit`, no link, no logo).

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
- `lib/ui/settings_page.dart`: offline login arrow uses `onPrimary`; "More from Ente", Help & Support, referrals, merchandise, Rate us and promo banners removed; Photovault footer added.
- `lib/ui/viewer/people/save_or_edit_person.dart`: badge icon uses `onPrimary`.
- `lib/ui/tools/editor/image_editor/image_editor_text_bar.dart`: the "Green" text swatch stays green (Encore `ok`).
- Hard-coded Ente greens routed through tokens: `lib/ui/account/two_factor_setup_page.dart`, `lib/ui/home/status_bar_widget.dart`, `lib/ui/map/map_gallery_tile_badge.dart`, `lib/ui/rituals/ritual_camera_page.dart`, `lib/ui/rituals/ritual_day_thumbnail.dart`, `lib/ui/rituals/ritual_page.dart`, `lib/ui/settings/backup/backup_item_card.dart`, `lib/ui/settings/storage_card_widget.dart`, `lib/ui/sharing/public_link_enabled_actions_widget.dart`, `lib/ui/social/comment_likes_bottom_sheet.dart`, `lib/ui/social/like_collection_selector_sheet.dart`, `lib/ui/social/likes_bottom_sheet.dart`, `lib/ui/social/widgets/comment_actions_popup.dart`, `lib/ui/social/widgets/comment_like_count_capsule.dart`, `lib/ui/social/widgets/feed_item_widget.dart`, `lib/ui/social/widgets/file_social_overlay.dart`, `lib/ui/viewer/file/video_download_progress_indicator.dart`, `lib/ui/viewer/people/memory_lane_banner.dart`, `lib/ui/viewer/people/person_gallery_suggestion.dart`, `lib/ui/wrapped/cards/badge_card_content.dart` (import in `lib/ui/wrapped/wrapped_viewer_page.dart`).

Photos app branding:
- `lib/ui/photovault/photovault_settings_footer.dart` (new): settings footer showing the app version only.
- `lib/ui/settings/about/about_us_page.dart`: About shows only the "Based on Ente" credit (`EncoreTokens.credit` in `encore_tokens.dart`).
- `lib/app.dart`: `MaterialApp.title` is "Photovault".
- `lib/services/notification_service.dart`: default notification channel name is "Photovault" (channel id unchanged).
- `lib/ui/home/grant_permissions_widget.dart`: Photovault wordmark instead of Ente's (wider logo box).
- `lib/ui/wrapped/wrapped_viewer_page.dart`: "Ente Rewind" title -> "Photovault Rewind".
- `assets/ente-branding.svg`, `assets/ente_io_green_black.svg`, `assets/ente_io_green.png`: Photovault wordmark (file names kept so code is untouched).
- `assets/qr_logo.png`, `assets/launcher_icon/*.png`: Photovault icon / glyph.
- `scripts/photovault/render_branding.js`, `scripts/photovault/wordmark.py` (new): asset generator.

Android:
- `android/app/src/main/res/values/strings.xml`, `src/debug/.../strings.xml`, `src/dev/.../strings.xml`: app label "Photovault".
- `android/app/src/main/res/values/colors.xml`: adaptive-icon background #1c1c1e.
- `android/app/src/main/res/values-v31/styles.xml`, `values-night-v31/styles.xml`: Android 12 splash background (Encore light/dark bg).
- `android/app/src/main/res/mipmap-*/{ic_launcher,launcher_icon,icon_*}.png`, `src/dev/res/mipmap-*/launcher_icon.png`: legacy launcher icons.
- `android/app/src/main/res/drawable-*/ic_*launcher_foreground.png`: adaptive-icon foregrounds (glyph) and monochrome layer.
- `android/app/src/main/res/drawable-*/splash.png`, `drawable*/android12splash.png`, `drawable*/background.png`: splash images and backgrounds.
- `android/app/src/main/res/drawable/notification_icon.png`: notification glyph.

iOS:
- `ios/Runner.xcodeproj/project.pbxproj`: `APP_DISPLAY_NAME` and extension `CFBundleDisplayName` -> Photovault (bundle ids unchanged).
- `ios/Runner/Info.plist`: brand name in permission prompts.
- `ios/Runner/Assets.xcassets/*.appiconset/*.png`: app icons (any/dark/tinted) for every icon set.
- `ios/Runner/Assets.xcassets/LaunchImage.imageset/*.png`, `LaunchBackground.imageset/*.png`: launch screen.

## Company material removed and build de-Googled

What we build is the de-Googled `independent` flavor (`fdroid` and `dev` are
kept; `playstore` is gone). Android `applicationId` is `app.photovault`
(`app.photovault.debug` for debug builds, no flavor suffix on `independent`),
per `contracts/apps.json`. The Kotlin/Java namespace stays `io.ente.photos` so
no native code moves.

### Server connection

Built against `contracts/server-info.md` and `contracts/apps.json`. The
contract model lives in ONE file, `lib/services/photovault/server_contract.dart`
(`/photovault/info` fields, `/ping` fallback, 1.5 s home probe, QR payload,
`_photovault._tcp` TXT keys); change that file when the contract changes.
API paths are Ente's, unchanged: only the base URL in Ente's
`EndpointConfig` is set.

- `lib/core/network/endpoint_config.dart`: the built-in server is
  `--dart-define=PHOTOVAULT_SERVER=<url>` (Ente's `endpoint` define still
  works as a fallback). Empty = none; Ente's `api.ente.com` is no longer a
  default. `isProduction` is always false, so no Ente-production-only path
  (Cloudflare upload worker, "faster uploads", free-plan upsells) is taken.
  Optional `--dart-define=PHOTOVAULT_SERVER_NAME=<label>` (contract).
- New, pure Dart (unit-tested in `test/photovault/`):
  `lib/services/photovault/server_contract.dart` (model, parsers),
  `server_address.dart` (address normaliser: `host:port`, http/https, IPv6,
  `photovault://connect` links; http by default for LAN-only names, https
  otherwise), `server_address_selector.dart` (home/public choice: home if it
  answers `/photovault/info` within 1.5 s with the same `publicUrl`, else
  public), `server_info_client.dart` (Connect request and error kinds:
  unreachable, TLS, not Photovault, HTTP error; a stock Ente server that only
  answers `/ping` is accepted as a "Compatible server").
- New, Flutter: `lib/services/photovault/photovault_server_service.dart`
  (saved servers and active server in SharedPreferences `photovault.*`;
  sets Ente's endpoint; re-checks home/public on start and on connectivity
  change), `server_discovery.dart` (mDNS `_photovault._tcp` with the pure-Dart
  `multicast_dns`, holding `ente_cast`'s Android multicast lock; Android
  only), `photovault_strings.dart` (English UI text, not in the ARB files).
- New UI: `lib/ui/photovault/connect_server_page.dart` ("Connect to your
  server": logo, address field, Connect -> name/version card -> Continue;
  saved servers; servers on this network; QR button; "Use without a server"
  for Ente's offline gallery), `server_qr_scan_page.dart` (Ente's
  `ente_qr_scanner`: CameraX + zxing-cpp, no Play services),
  `server_settings_page.dart` (Settings > Server: name, version, home/public
  address and which is in use, Switch server = Ente's logout, then the
  connect screen), `photovault_server_chip.dart` (landing-page line "Connected
  to X" with "Change server").
- `lib/ui/home/landing_page_widget.dart`: shows the connect screen while no
  server is active, then Ente's landing page unchanged except for the server
  line and hiding "Create account" when the server reports
  `signupsAllowed: false`. Ente's sign-in / sign-up pages are untouched.
- `lib/ui/settings_page.dart`: "Server" entry; the offline-mode "Log in" card
  asks for a server first when none is active.
- `lib/core/configuration.dart`: `logout()` keeps the `photovault.*` keys and
  Ente's `endpoint` key across its `SharedPreferences.clear()`, so servers
  survive logout.
- `lib/main.dart`: starts the server service after the service locator (both
  foreground and background init; background awaits the address choice, at
  most 5 s).
- `plugins/ente_cast/lib/ente_cast.dart`: exports `withMulticastLock`
  (the plugin already declares `CHANGE_WIFI_MULTICAST_STATE`).
- `pubspec.yaml`: `ente_qr_scanner` (workspace path package, already used by
  Ente Auth) and `multicast_dns` 0.3.3+1 (already in the lock file through
  `ente_cast`). No new native library: zxing-cpp is already shipped by
  `ente_qr`.
- Public album links: `lib/services/collections_service.dart` remembers the
  origin a shared link was opened from (`getSharedPublicAlbumOrigin`) and
  `lib/ui/viewer/gallery/gallery_app_bar_widget.dart` "Open in browser" uses it
  (falling back to the endpoint) instead of albums.ente.com.
- The 7-tap developer-settings dialog that used to set the endpoint is deleted.

### Telemetry, Firebase, Play services, billing

- `pubspec.yaml`: removed `firebase_core`, `firebase_messaging`,
  `in_app_purchase`, `in_app_purchase_storekit`, `sentry`, `sentry_flutter`,
  `rive`; `../../pubspec.lock` regenerated.
- `android/app/build.gradle`: google-services plugin removed, `playstore`
  flavor removed, `applicationId` set. `android/app/src/playstore/` deleted.
- `android/app/src/main/AndroidManifest.xml`: removed Sentry DSN, Firebase and
  `asset_statements` meta-data, `BILLING` permission, albums.ente.io/.com
  app links and the duck icon alias (the `ente://` scheme stays for passkeys).
  `res/values/strings.xml`: `asset_statements` removed.
- `android/app/src/main/play/` (store listings and screenshots) and
  `android/app/google-services.json` deleted.
- iOS: `ios/GoogleService-Info.plist` deleted and its four references removed
  from `ios/Runner.xcodeproj/project.pbxproj`; `ios/Runner/Runner.entitlements`
  drops Ente's web-credential and albums app-link domains;
  `ios/NotificationServiceExtension/NotificationService.swift` title is
  "Photovault". `Podfile.lock` and the Pods embed phases still list
  Firebase/Sentry until the next `pod install` on macOS.
- `lib/core/error-reporting/super_logging.dart`, `lib/main.dart`,
  `lib/core/configuration.dart`, `lib/utils/bg_task_utils.dart`,
  `lib/utils/background_tasks.dart`: Sentry, anonymous user id, StoreKit and
  the log-mailing trigger removed. `tunneled_transport.dart` deleted.
- `../../packages/logging/` (shared): Sentry removed; `sentryDsn`, `tunnel`,
  `setUserID` (no-op) etc. kept as API so Auth/Locker still compile.
- `lib/services/push_service.dart`: Ente's own F-Droid no-op stub (no FCM).
  `fdroid/overlay/`, `scripts/prepare_fdroid_source.sh`,
  `lib/services/account/purchase_update_listener.dart` and
  `integration_test/notification_lifecycle_test.dart` (tested FCM
  registration) deleted.
- `lib/ui/settings/security/security_settings_page.dart`,
  `lib/ui/settings/search/settings_search_registry.dart`: crash-reporting
  toggle removed.

- `../../packages/feature_flag/lib/src/remote_flags.dart` (shared, only
  Photos reads these): default `castUrl`, `embedUrl`, `customDomainCNAME` are
  empty instead of cast/embed/my.ente.com; `lib/ui/cast/pair_with_code.dart`
  falls back to `<endpoint>/cast` (per `contracts/server-env.md`).
- `lib/services/notification_service.dart`: channel description "Photovault alerts".

### Plans, subscriptions, payments

- `lib/ui/payment/`, `test/ui/payment/`, `assets/subscription/` deleted.
- `lib/services/account/billing_service.dart`: reduced to `clearCache()` and
  `launchFamilyPortal` (family stays as upstream).
- `lib/ui/family/family_plan_page.dart`: free-plan advert and price removed.
- `lib/ui/settings/storage_card_widget.dart`: shows used/total from
  `/users/details/v2` with no plan name; tap does nothing unless family.
- Subscribe/Upgrade buttons removed: `lib/ui/actions/collection/collection_sharing_actions.dart`,
  `collection_file_actions.dart`, `lib/ui/sharing/share_person_sheet.dart`,
  `lib/ui/sharing/manage_links_widget.dart`,
  `lib/ui/settings/account/account_settings_page.dart` (also "Export your data" help link).
- `lib/ui/account/password_entry_page.dart`, `lib/ui/home/landing_page_widget.dart`:
  the post-signup plan picker is skipped (goes straight home).
- `lib/ui/home/header_error_widget.dart`: storage-full banner opens Free up space.

### Ente links, support, promos, changelog

- Removed: Help & Support (`lib/ui/settings/support/`), referrals
  (`lib/ui/growth/`), Merchandise and Rate us (`lib/ui/settings_page.dart`),
  get-started and offline settings banners (`lib/ui/components/banners/`),
  "More from Ente", What's new / change log and update sheet
  (`lib/ui/notification/update/`, `lib/ui/settings/app_update_sheet.dart`,
  `lib/services/review_service.dart`), the Christmas banner/snow
  (`lib/ui/home/christmas/`), developer settings
  (`lib/ui/settings/developer_settings_*.dart`).
- `lib/services/update_service.dart`, `lib/ui/tabs/home_widget.dart`: no
  update check against Ente, change log never shown.
- `lib/ui/home/header_widget.dart`, `lib/ui/home/status_bar_widget.dart`,
  `lib/ui/home/christmas/christmas_utils.dart`: banners unhooked.
- Rate-us prompts replaced by a plain OK dialog:
  `lib/ui/viewer/gallery/gallery_app_bar_widget.dart`,
  `lib/ui/settings/backup/free_space_options.dart`.
- Links to ente.com removed: `lib/ui/account/email_entry_page.dart` (terms),
  `password_entry_page.dart` ("How it works"), `lib/ui/settings/ml/ml_consent_sheet.dart`,
  `lib/ui/settings/streaming/video_streaming_settings_page.dart`,
  `lib/ui/home/grant_permissions_widget.dart`, `lib/ui/account/two_factor_recovery_page.dart`
  (now "ask your server administrator"), `lib/utils/email_util.dart`
  (logs are exported/shared only; no "email support@ente.com").
- `lib/core/constants.dart`: Sentry DSNs, tunnel, GitHub discussions,
  web/family URLs removed.
- User-visible "Ente"/"via ente" text: `lib/ui/home/loading_photos_widget.dart`,
  `lib/ui/offline/empty/share.dart`, `lib/ui/sharing/public_link_enabled_actions_widget.dart`,
  `lib/services/social_notification_coordinator.dart`, `lib/ui/account/recovery_key_page.dart`,
  `lib/ui/tools/collage/collage_creator_page.dart`, `lib/utils/share_util.dart`,
  `lib/ui/map/tile/layers.dart` (User-Agent), `lib/ui/wrapped/*`,
  `lib/services/wrapped/builders/badge_selector.dart`, `lib/models/search/search_types.dart`,
  `lib/ui/settings/debug/debug_settings_page.dart`.
- `../../packages/account_deletion/lib/src/ui/delete_account_confirmation_step.dart`
  (shared): Ente Auth/Locker rows removed from the deletion summary.
- `../../packages/ente_components/assets/apps/photos.png` (all densities): Photovault icon.
- `../../packages/lock_screen/lib/auth_util.dart` (shared): app-lock error messages say "App lock" instead of "Ente App lock".
- `../../packages/strings/lib/l10n/arb/strings_*.arb`: server-neutral English
  for account deletion, verification-ID share, sharing limits, storage full,
  logs dialog, "Log in to Photovault"; new `askServerAdminToReset2fa`;
  ente.com stripped from share texts; remaining "Ente" forms and
  transliterations in ja, ko, hu, he, sr, et, gu, ur replaced. Translations whose meaning changed were deleted so
  they fall back to English.

### Mascot

- `assets/photovault_glyph.svg` and `lib/ui/photovault/photovault_glyph.dart`
  (new): single-colour glyph, theme coloured, never animated.
- Rive animations (`assets/*.riv`) deleted; `lib/ui/account/onboarding_page_scaffold.dart`,
  `lib/ui/home/landing_page_widget.dart`, `lib/ui/components/empty_state_illustration.dart`,
  `lib/ui/sharing/widgets/sharing_progress_sheet.dart`,
  `lib/ui/tools/similar_images_page.dart`, `lib/ui/wrapped/wrapped_rewind_banner_button.dart`
  draw the glyph or a flat colour instead.
- Duck PNG/SVG illustrations (`assets/ducky_*`, `assets/*-static.png`,
  `assets/rewind_badges/*`, `assets/rituals/*ducky*.svg`,
  `assets/photo_review/all_cleaned_up.svg`, etc.) are re-rendered by
  `scripts/photovault/render_mascot_replacements.js` (Playwright + Chrome);
  run it again after an upstream merge adds duck art.
- `assets/splash-screen-icon.png` (flutter_native_splash source) is the Photovault icon.
- Duck alternate app icon removed (`lib/ui/settings/app_icon_selection_screen.dart`,
  Android drawables/mipmap alias, iOS `IconDuckyHuggingE.appiconset`,
  `assets/launcher_icon/icon-ducky-hugging-e*.png`).

### Build, signing, tests and CI

- `android/app/build.gradle`: release signing still reads Ente's
  `android/key.properties` or `SIGNING_KEY_PATH` / `SIGNING_KEY_ALIAS` /
  `SIGNING_KEY_PASSWORD` / `SIGNING_STORE_PASSWORD`. It also reads a
  `key.properties` at `$PHOTOVAULT_KEY_PROPERTIES`. With no release key, release
  builds are signed with the debug key instead of failing. The Photovault
  keystore and its passwords live outside the repo (see the signing README
  in the project files); never commit them.
- `../../../.github/workflows/photovault-mobile.yml` (new): on push/PR touching
  `mobile/**` or `rust/**`, runs `flutter analyze` and `flutter test`, builds the
  arm64 `independent` release APK (signed when the `PHOTOVAULT_KEYSTORE_BASE64`,
  `PHOTOVAULT_KEYSTORE_PASSWORD`, `PHOTOVAULT_KEY_ALIAS` and `PHOTOVAULT_KEY_PASSWORD`
  secrets exist, otherwise debug-signed), checks its applicationId, signature
  and that no Firebase/Play services/Sentry/Billing classes are in the dex, and
  uploads it with `SHA256SUMS`.
- `lib/services/photovault/photovault_server_service.dart`: `overrideForTesting`
  (fake HTTP client / LAN discovery) for widget tests.
- Tests changed because of our patches: `test/ui/account/onboarding_pages_test.dart`
  (no terms checkbox), `test/ui/home/memories/full_screen_memory_test.dart` (uses a
  plain bottom sheet instead of the removed What's New sheet),
  `test/utils/delete_file_util_test.dart` ("Delete from Photovault").
- `test/photovault/screenshots_test.dart` (new): renders the connect screen
  (empty, saved and found servers, server answered, unreachable), the top of
  Settings (storage card and Server entry), Settings > Server and About, in light
  and dark with the real themes and bundled fonts. With
  `PHOTOVAULT_SCREENSHOT_DIR=<dir>` it writes `<screen>-<light|dark>.png` there.
- `lib/main.dart`, `lib/ui/account/onboarding_page_scaffold.dart`: analyzer
  fixes (unused imports left by the telemetry removal, a missing `const`).

## Known gaps

- Still contacting Ente-run hosts for features we keep: ML model downloads
  (models.ente.com, set in `rust/crates/ml/src/assets.rs`), memories/rewind music
  (music.ente.com, `lib/services/memories/memory_music_service.dart`). The map uses
  OpenStreetMap tiles. Self-hosting these needs a later change.
- Server connection: LAN discovery runs on Android only (iOS needs Apple's
  multicast entitlement or a native Bonjour browser, and `_photovault._tcp`
  in `NSBonjourServices`). The connect/settings text is English only.
- Unused ARB keys still mention Ente Auth/Locker, referrals and support e-mail;
  they are never displayed by Photos (see `PHOTOVAULT-GREP.md`).
- Shared `ente_ui`/`ente_utils` helpers still name support@ente.com; Photos
  does not call those paths.
- Chromecast uses Ente's registered receiver app id (`F5BCEC64` in
  `plugins/ente_cast/lib/src/service.dart`), which loads Ente's hosted cast
  page. Pairing by code goes through the server and `<server>/cast` instead
  (not tested here). Replacing
  it needs a Photovault receiver registered with Google.
- Some non-mascot illustrations keep Ente's green (e.g. `assets/on_ente.png`,
  `lock_screen_icon.png`, `recovery_key.png`, rewind badge backdrops).
- The alternate app icons (Settings > App icon) all show the Photovault icon.
- `PHOTOVAULT-GREP.md` (generated by a final grep) lists every remaining "ente"/telemetry/store hit and why it is acceptable.
