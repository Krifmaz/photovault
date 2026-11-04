# Photovault: final grep of the Photos app and shared packages

Generated for the Photovault fork of `mobile/apps/photos`. Patterns, run case-insensitively with ripgrep:

`ente.io`, `ente.com`, `\bente\b` (word: `ente_ui`, `EnteFoo`, `io.ente.x` package names do not match as words unless separated by dots), `duck`, `mascot`, `referral`, `merch`, `rate us`, `subscription`, `sentry`, `firebase`, `play.google`, `apps.apple`.

Scope: `apps/photos/{lib,assets,android,ios,pubspec.yaml}` and `mobile/packages/*` (all shared packages, including the ARB strings). Excluded: build outputs, `.dart_tool`, generated `strings_localizations*.dart` (regenerated from the ARB files), `lib/generated/` protobufs, `GeneratedPluginRegistrant`, binary images (images were checked visually instead).

Rule used: internal identifiers, class/package names, API paths and fields, comments, log messages and the About credit are acceptable; anything a user can see is not. Hits that were user-visible have been fixed in this branch; what is listed below is what remains, each with why it is acceptable or where it is a known gap.

## Update: second strip pass

The totals and per-file lines below are from the first pass. Since then (see
"Ente hosts the app contacted directly" in `PHOTOVAULT.md`) these hits are
gone, so their entries below are superseded:

- every `"support@ente.com"` argument in `apps/photos/lib` (now `supportEmail`,
  which is empty) and in `packages/{accounts,ui}` (now `''`, which hides the
  e-mail button in `ente_utils`' `sendLogs`);
- `apps/photos/lib/core/constants.dart` production/legacy/uploader endpoint
  values, `apps/photos/lib/module/download/file_url.dart` CDN worker URLs,
  `apps/photos/lib/services/memories/memory_music_service.dart` (music is no
  longer fetched), `apps/photos/lib/ui/tabs/home_widget.dart` albums.ente.*
  host check;
- `packages/ui/lib/components/settings/{about_settings_section,app_engagement_section,more_from_ente_footer,social_icons_row}.dart`,
  `packages/ui/lib/utils/dialog_util.dart`, `packages/utils/lib/email_util.dart`
  (`openSupportPage`), `packages/accounts/lib/pages/*` terms/privacy/
  architecture links, `packages/lock_screen/lib/ui/local_authentication_unavailable_dialog.dart`
  help link, `packages/ente_components/example/lib/main.dart` sample link;
- Rust: the Photos app downloads location data from its server
  (`<endpoint>/static-assets/location/v2`), not from Ente's assets CDN.

Still remaining on purpose: ML models from models.ente.com (pending the
owner's decision), the Chromecast receiver app id, `packages/configuration`
(Auth/Locker default endpoint), `packages/legacy` (not compiled into Photos),
podspec metadata, PHOTOVAULT comments and upstream comments.

## Totals (remaining hits)

| pattern | hits | files |
|---|---|---|
| `ente.io` | 19 | 15 |
| `ente.com` | 352 | 85 |
| `ente (word)` | 1290 | 364 |
| `duck` | 63 | 28 |
| `mascot` | 13 | 12 |
| `referral` | 464 | 51 |
| `merch` | 39 | 37 |
| `rate us` | 7 | 5 |
| `subscription` | 1260 | 162 |
| `sentry` | 41 | 17 |
| `firebase` | 55 | 8 |
| `play.google` | 0 | 0 |
| `apps.apple` | 0 | 0 |

`play.google` and `apps.apple` have no hits (store links removed with Rate us / update sheet / store listings).

## Known user-visible gaps (not fixed, by design or needing other work)

- Chromecast receiver app id `F5BCEC64` (`apps/photos/plugins/ente_cast/lib/src/service.dart`, outside the grep scope) loads Ente's hosted cast receiver; needs a Photovault receiver registered with Google. Pair-by-code uses `<server>/cast`.
- Network phone-home kept for a feature that must stay, pending the owner's decision: ML models (models.ente.com, set in `rust/crates/ml/src/assets.rs`). OpenStreetMap tiles for the map. (Memories/Rewind music is no longer fetched.)
- iOS `Podfile.lock` / `project.pbxproj` embed phases still name Firebase and Sentry frameworks until `pod install` is re-run on macOS (the Dart dependencies are gone). iOS share-extension `NSHumanReadableCopyright` still reads "Ente Technologies" (Ente's copyright on the code; not normally displayed).
- The "Contact support" / "Raise ticket" buttons in a few error dialogs remain; they now only export logs for the user to share (no Ente address). The error texts next to them no longer speak as a vendor ("contact our support team", "reach out to us", "Your plan already supports a family"): see the strings list in `PHOTOVAULT.md`.

## Photos app (`apps/photos`)

- `apps/photos/android/app/build.gradle` (`ente (word)`×6, `firebase`×1): Kotlin `namespace = "io.ente.photos"` (kept), Ente's ONNX Runtime AAR repository/group (ML dependency), PHOTOVAULT comments; internal.
- `apps/photos/android/app/src/debug/AndroidManifest.xml` (`ente (word)`×1): `package="io.ente.photos"` namespace, `ente` URI scheme (passkey redirect) and PHOTOVAULT comments; the fdroid manifest only strips Sentry meta-data (inert now). Internal.
- `apps/photos/android/app/src/dev/AndroidManifest.xml` (`ente (word)`×1): `package="io.ente.photos"` namespace, `ente` URI scheme (passkey redirect) and PHOTOVAULT comments; the fdroid manifest only strips Sentry meta-data (inert now). Internal.
- `apps/photos/android/app/src/fdroid/AndroidManifest.xml` (`ente (word)`×1, `sentry`×2): `package="io.ente.photos"` namespace, `ente` URI scheme (passkey redirect) and PHOTOVAULT comments; the fdroid manifest only strips Sentry meta-data (inert now). Internal.
- `apps/photos/android/app/src/independent/AndroidManifest.xml` (`ente (word)`×1): `package="io.ente.photos"` namespace, `ente` URI scheme (passkey redirect) and PHOTOVAULT comments; the fdroid manifest only strips Sentry meta-data (inert now). Internal.
- `apps/photos/android/app/src/main/AndroidManifest.xml` (`ente.io`×1, `ente.com`×1, `ente (word)`×5, `duck`×1, `mascot`×1, `sentry`×1, `firebase`×1): `package="io.ente.photos"` namespace, `ente` URI scheme (passkey redirect) and PHOTOVAULT comments; the fdroid manifest only strips Sentry meta-data (inert now). Internal.
- `apps/photos/android/app/src/main/kotlin/io/ente/photos/AdbDiagnostics.kt` (`ente (word)`×6): Kotlin package `io.ente.photos` (namespace kept so native code is untouched); AdbDiagnostics "Ente logs" text is adb-only developer output.
- `apps/photos/android/app/src/main/kotlin/io/ente/photos/EnteAlbumsWidgetProvider.kt` (`ente (word)`×1): Kotlin package `io.ente.photos` (namespace kept so native code is untouched); AdbDiagnostics "Ente logs" text is adb-only developer output.
- `apps/photos/android/app/src/main/kotlin/io/ente/photos/EnteApplication.kt` (`ente (word)`×4): Kotlin package `io.ente.photos` (namespace kept so native code is untouched); AdbDiagnostics "Ente logs" text is adb-only developer output.
- `apps/photos/android/app/src/main/kotlin/io/ente/photos/EnteMemoryWidgetProvider.kt` (`ente (word)`×1): Kotlin package `io.ente.photos` (namespace kept so native code is untouched); AdbDiagnostics "Ente logs" text is adb-only developer output.
- `apps/photos/android/app/src/main/kotlin/io/ente/photos/EntePeopleWidgetProvider.kt` (`ente (word)`×1): Kotlin package `io.ente.photos` (namespace kept so native code is untouched); AdbDiagnostics "Ente logs" text is adb-only developer output.
- `apps/photos/android/app/src/main/kotlin/io/ente/photos/ForegroundHeartbeat.kt` (`ente (word)`×1): Kotlin package `io.ente.photos` (namespace kept so native code is untouched); AdbDiagnostics "Ente logs" text is adb-only developer output.
- `apps/photos/android/app/src/main/kotlin/io/ente/photos/MainActivity.kt` (`ente (word)`×3): Kotlin package `io.ente.photos` (namespace kept so native code is untouched); AdbDiagnostics "Ente logs" text is adb-only developer output.
- `apps/photos/android/app/src/main/kotlin/io/ente/photos/MediaPickerActivity.kt` (`ente (word)`×1): Kotlin package `io.ente.photos` (namespace kept so native code is untouched); AdbDiagnostics "Ente logs" text is adb-only developer output.
- `apps/photos/android/app/src/main/kotlin/io/ente/photos/MediaReviewActivity.kt` (`ente (word)`×1): Kotlin package `io.ente.photos` (namespace kept so native code is untouched); AdbDiagnostics "Ente logs" text is adb-only developer output.
- `apps/photos/android/app/src/main/kotlin/io/ente/photos/WallpaperActivity.kt` (`ente (word)`×2): Kotlin package `io.ente.photos` (namespace kept so native code is untouched); AdbDiagnostics "Ente logs" text is adb-only developer output.
- `apps/photos/android/app/src/main/kotlin/io/ente/photos/WidgetImageDecoder.kt` (`ente (word)`×1): Kotlin package `io.ente.photos` (namespace kept so native code is untouched); AdbDiagnostics "Ente logs" text is adb-only developer output.
- `apps/photos/android/app/src/main/res/values/strings.xml` (`ente (word)`×1): PHOTOVAULT comment only.
- `apps/photos/android/app/src/main/res/xml/network_security_config.xml` (`ente.io`×1, `ente.com`×1, `ente (word)`×2): Forces TLS for Ente domains (upstream hardening); causes no traffic. Internal.
- `apps/photos/android/app/src/profile/AndroidManifest.xml` (`ente (word)`×1): `package="io.ente.photos"` namespace, `ente` URI scheme (passkey redirect) and PHOTOVAULT comments; the fdroid manifest only strips Sentry meta-data (inert now). Internal.
- `apps/photos/android/build.gradle` (`ente (word)`×1): Shared Gradle include file name `ente-android.gradle`; internal.
- `apps/photos/android/gradle/verification-metadata.xml` (`ente (word)`×3): Gradle verification entry for Ente's ONNX Runtime artifact (ML dependency); internal.
- `apps/photos/assets/ducky_cleaning_static.svg` (`ente (word)`×1, `duck`×1, `mascot`×1): PHOTOVAULT header comment explaining the glyph replaced the mascot; artwork has no mascot.
- `apps/photos/assets/ente-branding.svg` (`ente (word)`×1): Photovault wordmark; the hit is the PHOTOVAULT header comment.
- `apps/photos/assets/ente_io_green_black.svg` (`ente (word)`×1): Photovault wordmark; the hit is the PHOTOVAULT header comment.
- `apps/photos/assets/photo_review/all_cleaned_up.svg` (`ente (word)`×1, `duck`×1, `mascot`×1): PHOTOVAULT header comment explaining the glyph replaced the mascot; artwork has no mascot.
- `apps/photos/assets/photovault_glyph.svg` (`ente (word)`×1, `duck`×1, `mascot`×1): PHOTOVAULT header comment explaining the glyph replaced the mascot; artwork has no mascot.
- `apps/photos/assets/rituals/ducky_ritual.svg` (`ente (word)`×1, `duck`×1, `mascot`×1): PHOTOVAULT header comment explaining the glyph replaced the mascot; artwork has no mascot.
- `apps/photos/assets/rituals/ritual_ducky_share.svg` (`ente (word)`×1, `duck`×1, `mascot`×1): PHOTOVAULT header comment explaining the glyph replaced the mascot; artwork has no mascot.
- `apps/photos/ios/EnteAlbumWidget/EnteAlbumWidget.swift` (`ente (word)`×1): App-group / keychain / bundle identifiers `group.io.ente.frame.*`, `io.ente.*` (kept so extensions keep working) and PHOTOVAULT comments; internal.
- `apps/photos/ios/EnteAlbumWidgetExtension.entitlements` (`ente (word)`×1): App-group / keychain / bundle identifiers `group.io.ente.frame.*`, `io.ente.*` (kept so extensions keep working) and PHOTOVAULT comments; internal.
- `apps/photos/ios/EnteMemoryWidget/EnteMemoryWidget.swift` (`ente (word)`×1): App-group / keychain / bundle identifiers `group.io.ente.frame.*`, `io.ente.*` (kept so extensions keep working) and PHOTOVAULT comments; internal.
- `apps/photos/ios/EnteMemoryWidgetExtension.entitlements` (`ente (word)`×1): App-group / keychain / bundle identifiers `group.io.ente.frame.*`, `io.ente.*` (kept so extensions keep working) and PHOTOVAULT comments; internal.
- `apps/photos/ios/EnteMemoryWidgetExtensionDebug.entitlements` (`ente (word)`×1): App-group / keychain / bundle identifiers `group.io.ente.frame.*`, `io.ente.*` (kept so extensions keep working) and PHOTOVAULT comments; internal.
- `apps/photos/ios/EntePeopleWidget/EntePeopleWidget.swift` (`ente (word)`×1): App-group / keychain / bundle identifiers `group.io.ente.frame.*`, `io.ente.*` (kept so extensions keep working) and PHOTOVAULT comments; internal.
- `apps/photos/ios/EntePeopleWidgetExtension.entitlements` (`ente (word)`×1): App-group / keychain / bundle identifiers `group.io.ente.frame.*`, `io.ente.*` (kept so extensions keep working) and PHOTOVAULT comments; internal.
- `apps/photos/ios/NotificationServiceExtension/NotificationKeyStore.swift` (`ente (word)`×2): App-group / keychain / bundle identifiers `group.io.ente.frame.*`, `io.ente.*` (kept so extensions keep working) and PHOTOVAULT comments; internal.
- `apps/photos/ios/NotificationServiceExtension/NotificationServiceExtension.entitlements` (`ente (word)`×1): App-group / keychain / bundle identifiers `group.io.ente.frame.*`, `io.ente.*` (kept so extensions keep working) and PHOTOVAULT comments; internal.
- `apps/photos/ios/NotificationServiceExtension/ente_notification_uniffi.podspec` (`ente (word)`×3): Podspec metadata (summary/author/homepage); not shown.
- `apps/photos/ios/Podfile` (`ente (word)`×1): Comment about Ente's ONNX Runtime prebuilt (ML dependency).
- `apps/photos/ios/Podfile.lock` (`sentry`×9, `firebase`×36): Stale CocoaPods lock listing Firebase/Sentry pods; regenerated by `pod install` now that the Dart plugins are gone (iOS not built here). Known gap.
- `apps/photos/ios/Runner.xcodeproj/project.pbxproj` (`ente (word)`×27, `sentry`×4, `firebase`×10): Bundle ids `io.ente.frame*`, app-group ids, extension copyright, and stale Pods embed entries for Firebase/Sentry (regenerated by `pod install`). Not shown in the Android build; known iOS gap.
- `apps/photos/ios/Runner/AppDelegate.swift` (`ente (word)`×5): App-group / keychain / bundle identifiers `group.io.ente.frame.*`, `io.ente.*` (kept so extensions keep working) and PHOTOVAULT comments; internal.
- `apps/photos/ios/Runner/ForegroundHeartbeat.swift` (`ente (word)`×1): App-group / keychain / bundle identifiers `group.io.ente.frame.*`, `io.ente.*` (kept so extensions keep working) and PHOTOVAULT comments; internal.
- `apps/photos/ios/Runner/Info.plist` (`ente (word)`×6, `firebase`×1): BGTask identifiers, bundle URL name and `ente` URL scheme (passkeys), inert `FIREBASE_ANALYTICS_COLLECTION_DEACTIVATED` flag; internal.
- `apps/photos/ios/Runner/NotificationKeyPlugin.swift` (`ente (word)`×1): App-group / keychain / bundle identifiers `group.io.ente.frame.*`, `io.ente.*` (kept so extensions keep working) and PHOTOVAULT comments; internal.
- `apps/photos/ios/Runner/Runner.entitlements` (`ente.io`×1, `ente (word)`×5): App-group / keychain / bundle identifiers `group.io.ente.frame.*`, `io.ente.*` (kept so extensions keep working) and PHOTOVAULT comments; internal.
- `apps/photos/ios/ShareExtension/ShareExtension.entitlements` (`ente (word)`×1): App-group / keychain / bundle identifiers `group.io.ente.frame.*`, `io.ente.*` (kept so extensions keep working) and PHOTOVAULT comments; internal.
- `apps/photos/lib/app.dart` (`subscription`×14): only identifiers (class/field/variable names such as `StreamSubscription`, `EnteFile`, `Subscription` model); internal.
- `apps/photos/lib/core/configuration.dart` (`ente (word)`×1, `sentry`×1):
  - L114 `_documentsDirectory + "/ente-shared-media";`: Internal temp-file / key prefix; not shown.
  - remaining hits are comments; internal.
- `apps/photos/lib/core/constants.dart` (`ente.io`×1, `ente.com`×4, `ente (word)`×9, `sentry`×1):
  - L39 `const kLegacyProductionEndpoint = 'https://api.ente.io';`: Upstream production endpoint constants; the server comes from dart-define `PHOTOVAULT_SERVER` (or `endpoint`) / the connect screen via `setEndpoint()`; never a default.
  - L38 `const kDefaultProductionEndpoint = 'https://api.ente.com';`: Upstream production endpoint constants; the server comes from dart-define `PHOTOVAULT_SERVER` (or `endpoint`) / the connect screen via `setEndpoint()`; never a default.
  - L40 `const kUploadProxyEndpoint = 'https://uploader.ente.com';`: Upstream production endpoint constants; the server comes from dart-define `PHOTOVAULT_SERVER` (or `endpoint`) / the connect screen via `setEndpoint()`; never a default.
  - L34 `const supportEmail = 'support@ente.com';`: Unused constant kept for upstream compatibility; nothing displays it.
  - L23 `const String sharedMediaIdentifier = 'ente-shared-media://';`: Internal temp-file / key prefix; not shown.
  - L65 `const iOSGroupIDMemory = "group.io.ente.frame.EnteMemoryWidget";`: Platform identifier (namespace, bundle id, channel, task or app-group id); internal.
  - remaining hits are comments; internal.
- `apps/photos/lib/core/error-reporting/super_logging.dart` (`sentry`×4): only comments; internal.
- `apps/photos/lib/core/errors.dart` (`subscription`×3): only identifiers (class/field/variable names such as `StreamSubscription`, `EnteFile`, `Subscription` model); internal.
- `apps/photos/lib/core/network/endpoint_config.dart` (`ente (word)`×1): only comments; internal.
- `apps/photos/lib/core/network/network.dart` (`subscription`×4): only identifiers (class/field/variable names such as `StreamSubscription`, `EnteFile`, `Subscription` model); internal.
- `apps/photos/lib/db/collections_db.dart` (`ente (word)`×1):
  - L10 `static const _databaseName = "ente.collections.db";`: On-device database/index file name; internal.
- `apps/photos/lib/db/file_updation_db.dart` (`ente (word)`×1):
  - L7 `static const _databaseName = "ente.files_migration.db";`: On-device database/index file name; internal.
- `apps/photos/lib/db/files_db.dart` (`ente (word)`×1):
  - L32 `static const _databaseName = "ente.files.db";`: On-device database/index file name; internal.
- `apps/photos/lib/db/gallery_downloads_db.dart` (`ente (word)`×1):
  - L7 `static const _databaseName = "ente.gallery_downloads.db";`: On-device database/index file name; internal.
- `apps/photos/lib/db/ignored_files_db.dart` (`ente (word)`×2):
  - L10 `static const _databaseName = "ente.ignored_files.db";`: On-device database/index file name; internal.
  - remaining hits are comments; internal.
- `apps/photos/lib/db/memories_db.dart` (`ente (word)`×2):
  - L8 `static const _databaseName = "ente.memories.db";`: On-device database/index file name; internal.
  - L30 `dbName: "ente.memories.offline.db",`: On-device database/index file name; internal.
- `apps/photos/lib/db/memory_shares_db.dart` (`ente (word)`×1):
  - L6 `static const _databaseName = "ente.memory_shares.db";`: On-device database/index file name; internal.
- `apps/photos/lib/db/ml/dart_db.dart` (`ente (word)`×2):
  - L41 `String databaseName = "ente.ml.db",`: On-device database/index file name; internal.
  - L54 `databaseName: "ente.ml.offline.db",`: On-device database/index file name; internal.
- `apps/photos/lib/db/ml/db_model_mappers.dart` (`ente (word)`×1): only package imports (`ente_*` packages); internal.
- `apps/photos/lib/db/ml/ml_data_db_orchestration.dart` (`ente (word)`×1): only package imports (`ente_*` packages); internal.
- `apps/photos/lib/db/ml/rust_db.dart` (`ente (word)`×2):
  - L38 `String databaseName = "ente.ml.db",`: On-device database/index file name; internal.
  - L50 `databaseName: "ente.ml.offline.db",`: On-device database/index file name; internal.
- `apps/photos/lib/db/ml/usearch_clip_vector_db.dart` (`ente (word)`×2):
  - L32 `"ente.ml.vectordb.clip.usearch",`: On-device database/index file name; internal.
  - L36 `"ente.ml.offline.vectordb.clip.usearch",`: On-device database/index file name; internal.
- `apps/photos/lib/db/ml/usearch_cluster_centroid_vector_db.dart` (`ente (word)`×2):
  - L34 `"ente.ml.vectordb.cluster_centroid.usearch",`: On-device database/index file name; internal.
  - L40 `"ente.ml.offline.vectordb.cluster_centroid.usearch",`: On-device database/index file name; internal.
- `apps/photos/lib/db/offline_files_db.dart` (`ente (word)`×1):
  - L11 `static const _databaseName = "ente.offline_files.db";`: On-device database/index file name; internal.
- `apps/photos/lib/db/photo_review_db.dart` (`ente (word)`×1):
  - L8 `static const _databaseName = "ente.photo_review.db";`: On-device database/index file name; internal.
- `apps/photos/lib/db/social_db.dart` (`ente (word)`×1):
  - L13 `static const _databaseName = "ente.social.db";`: On-device database/index file name; internal.
- `apps/photos/lib/db/trash_db.dart` (`ente (word)`×1):
  - L11 `static const _databaseName = "ente.trash.db";`: On-device database/index file name; internal.
- `apps/photos/lib/db/upload_locks_db.dart` (`ente (word)`×1, `sentry`×1):
  - L11 `static const _databaseName = "ente.upload_locks.db";`: On-device database/index file name; internal.
  - remaining hits are identifiers (class/field/variable names such as `StreamSubscription`, `EnteFile`, `Subscription` model); internal.
- `apps/photos/lib/events/subscription_purchased_event.dart` (`subscription`×1): only identifiers (class/field/variable names such as `StreamSubscription`, `EnteFile`, `Subscription` model); internal.
- `apps/photos/lib/gateways/billing/billing_gateway.dart` (`subscription`×14):
  - L23 `"/billing/verify-subscription",`: Ente API path/JSON field (billing, user details); internal.
  - L30 `return Subscription.fromMap(response.data["subscription"]);`: Ente API path/JSON field (billing, user details); internal.
  - L40 `final response = await _enteDio.get("/billing/subscription");`: Ente API path/JSON field (billing, user details); internal.
  - L41 `return Subscription.fromMap(response.data["subscription"]);`: Ente API path/JSON field (billing, user details); internal.
  - L45 `final response = await _enteDio.post("/billing/stripe/cancel-subscription");`: Ente API path/JSON field (billing, user details); internal.
  - L46 `return Subscription.fromMap(response.data["subscription"]);`: Ente API path/JSON field (billing, user details); internal.
  - L51 `"/billing/stripe/activate-subscription",`: Ente API path/JSON field (billing, user details); internal.
  - L53 `return Subscription.fromMap(response.data["subscription"]);`: Ente API path/JSON field (billing, user details); internal.
  - remaining hits are package imports (`ente_*` packages), identifiers (class/field/variable names such as `StreamSubscription`, `EnteFile`, `Subscription` model); internal.
- `apps/photos/lib/gateways/billing/models/subscription.dart` (`subscription`×6): only identifiers (class/field/variable names such as `StreamSubscription`, `EnteFile`, `Subscription` model); internal.
- `apps/photos/lib/gateways/storage_bonus/models/bonus.dart` (`referral`×1):
  - L29 `static Set<String> signUpBonusTypes = {'SIGN_UP', 'REFERRAL'};`: Ente API path/JSON field (storage bonus / referral source); internal, the referral UI is deleted.
- `apps/photos/lib/gateways/storage_bonus/models/storage_bonus.dart` (`referral`×12):
  - L108 `json["referralStats"].map((x) => ReferralStat.fromJson(x)),`: Ente API path/JSON field (storage bonus / referral source); internal, the referral UI is deleted.
  - L117 `"referralStats": List<dynamic>.from(referralStats.map((x) => x.toJson())),`: Ente API path/JSON field (storage bonus / referral source); internal, the referral UI is deleted.
  - remaining hits are identifiers (class/field/variable names such as `StreamSubscription`, `EnteFile`, `Subscription` model); internal.
- `apps/photos/lib/gateways/storage_bonus/storage_bonus_gateway.dart` (`referral`×5):
  - L10 `final response = await _enteDio.get("/storage-bonus/referral-view");`: Ente API path/JSON field (storage bonus / referral source); internal, the referral UI is deleted.
  - L15 `return _enteDio.post("/storage-bonus/referral-claim?code=$code");`: Ente API path/JSON field (storage bonus / referral source); internal, the referral UI is deleted.
  - remaining hits are identifiers (class/field/variable names such as `StreamSubscription`, `EnteFile`, `Subscription` model); internal.
- `apps/photos/lib/main.dart` (`ente.com`×1, `ente (word)`×4, `mascot`×1, `sentry`×1, `firebase`×3):
  - L507 `appLogoAsset: 'assets/ente-branding.svg',`: Asset file name kept; the file contains the Photovault wordmark.
  - L770 `await _sync('firebaseBgSyncMainEngine');`: Background-task log label; internal.
  - L783 `await _init(true, via: 'firebasePush');`: Background-task log label; internal.
  - L788 `await _sync('firebaseBgSyncNoActiveProcess');`: Background-task log label; internal.
  - remaining hits are comments; internal.
- `apps/photos/lib/models/user_details.dart` (`subscription`×7):
  - L126 `Subscription.fromMap(map['subscription']),`: Ente API path/JSON field (billing, user details); internal.
  - L140 `'subscription': subscription.toMap(),`: Ente API path/JSON field (billing, user details); internal.
  - remaining hits are package imports (`ente_*` packages), identifiers (class/field/variable names such as `StreamSubscription`, `EnteFile`, `Subscription` model); internal.
- `apps/photos/lib/module/download/file_url.dart` (`ente.com`×4, `ente (word)`×4):
  - L60 `: "https://files.ente.com/?fileID=$fileID";`: Ente CDN worker URLs, only used when talking to Ente production (`isProduction` is false); dead path.
  - L65 `: "https://public-albums.ente.com/download/?fileID=$fileID";`: Ente CDN worker URLs, only used when talking to Ente production (`isProduction` is false); dead path.
  - L70 `: "https://thumbnails.ente.com/?fileID=$fileID";`: Ente CDN worker URLs, only used when talking to Ente production (`isProduction` is false); dead path.
  - L75 `: "https://public-albums.ente.com/preview/?fileID=$fileID";`: Ente CDN worker URLs, only used when talking to Ente production (`isProduction` is false); dead path.
- `apps/photos/lib/module/download/gallery.dart` (`ente (word)`×1): only comments; internal.
- `apps/photos/lib/module/download/gallery_download_queue_service.dart` (`subscription`×18): only identifiers (class/field/variable names such as `StreamSubscription`, `EnteFile`, `Subscription` model); internal.
- `apps/photos/lib/module/upload/service/file_uploader.dart` (`subscription`×7): only identifiers (class/field/variable names such as `StreamSubscription`, `EnteFile`, `Subscription` model); internal.
- `apps/photos/lib/module/upload/service/multipart.dart` (`sentry`×1): only identifiers (class/field/variable names such as `StreamSubscription`, `EnteFile`, `Subscription` model); internal.
- `apps/photos/lib/module/upload/service/upload_transport.dart` (`subscription`×3): only identifiers (class/field/variable names such as `StreamSubscription`, `EnteFile`, `Subscription` model); internal.
- `apps/photos/lib/service_locator.dart` (`ente (word)`×1): only comments; internal.
- `apps/photos/lib/services/account/billing_service.dart` (`subscription`×1): only comments; internal.
- `apps/photos/lib/services/account/user_service.dart` (`referral`×3, `subscription`×8):
  - L49 `static const kReferralSource = "referral_source";`: Ente API path/JSON field (storage bonus / referral source); internal, the referral UI is deleted.
  - remaining hits are identifiers (class/field/variable names such as `StreamSubscription`, `EnteFile`, `Subscription` model); internal.
- `apps/photos/lib/services/collections_service.dart` (`ente (word)`×1, `subscription`×4): only comments, identifiers (class/field/variable names such as `StreamSubscription`, `EnteFile`, `Subscription` model); internal.
- `apps/photos/lib/services/favorites_service.dart` (`subscription`×4, `sentry`×2): only identifiers (class/field/variable names such as `StreamSubscription`, `EnteFile`, `Subscription` model); internal.
- `apps/photos/lib/services/home_widget_service.dart` (`ente (word)`×1):
  - L86 `qualifiedAndroidName: 'io.ente.photos.$androidClass',`: Platform identifier (namespace, bundle id, channel, task or app-group id); internal.
- `apps/photos/lib/services/library_sharing_service.dart` (`ente (word)`×1, `subscription`×6):
  - L571 `throw StateError('No Ente public key found for recipient');`: Log/exception message for developers; not shown in UI.
  - remaining hits are identifiers (class/field/variable names such as `StreamSubscription`, `EnteFile`, `Subscription` model); internal.
- `apps/photos/lib/services/machine_learning/compute_controller.dart` (`subscription`×4): only comments, identifiers (class/field/variable names such as `StreamSubscription`, `EnteFile`, `Subscription` model); internal.
- `apps/photos/lib/services/machine_learning/face_ml/face_clustering/face_clustering_service.dart` (`ente (word)`×1): only package imports (`ente_*` packages); internal.
- `apps/photos/lib/services/machine_learning/face_ml/feedback/cluster_feedback.dart` (`ente (word)`×5):
  - L84 `"Person suggestions require an Ente account and should not run in local gallery mode.",`: Log/exception message for developers; not shown in UI.
  - L88 `"Skipping getSuggestionForPerson in local gallery mode because person suggestions require `: Log/exception message for developers; not shown in UI.
  - L1437 `"Fast person suggestions require an Ente account and should not run in local gallery mode.`: Log/exception message for developers; not shown in UI.
  - L1441 `"Skipping _getFastSuggestions in local gallery mode because person suggestions require an `: Log/exception message for developers; not shown in UI.
  - remaining hits are comments; internal.
- `apps/photos/lib/services/machine_learning/face_ml/feedback/dart_cluster_averages.dart` (`ente (word)`×1): only package imports (`ente_*` packages); internal.
- `apps/photos/lib/services/machine_learning/face_ml/feedback/dart_face_distances.dart` (`ente (word)`×1): only package imports (`ente_*` packages); internal.
- `apps/photos/lib/services/machine_learning/ml_model_download_service.dart` (`subscription`×9): only comments, identifiers (class/field/variable names such as `StreamSubscription`, `EnteFile`, `Subscription` model); internal.
- `apps/photos/lib/services/memories/memory_music_service.dart` (`ente.com`×1, `ente (word)`×1):
  - L12 `const _manifestURL = "https://music.ente.com/tracks.json";`: Memories/Rewind music manifest: phone-home for a kept feature (documented gap).
- `apps/photos/lib/services/notification_key_service.dart` (`ente (word)`×1):
  - L11 `static const _channel = MethodChannel('io.ente.frame.notifications');`: Platform identifier (namespace, bundle id, channel, task or app-group id); internal.
- `apps/photos/lib/services/notification_service.dart` (`ente (word)`×9):
  - L309 `'io.ente.frame.notifications',`: Platform identifier (namespace, bundle id, channel, task or app-group id); internal.
  - L320 `String channelID = "io.ente.photos",`: Platform identifier (namespace, bundle id, channel, task or app-group id); internal.
  - L356 `const channelID = "io.ente.photos.background.debug";`: Platform identifier (namespace, bundle id, channel, task or app-group id); internal.
  - L383 `String channelID = "io.ente.photos",`: Platform identifier (namespace, bundle id, channel, task or app-group id); internal.
  - L322 `String payload = "ente://home",`: Internal `ente://` deep-link scheme (passkey redirect, notification routing); never shown.
  - L375 `payload: "ente://home",`: Internal `ente://` deep-link scheme (passkey redirect, notification routing); never shown.
  - L385 `String payload = "ente://home",`: Internal `ente://` deep-link scheme (passkey redirect, notification routing); never shown.
  - L331 `channelDescription: 'Photovault alerts', // PHOTOVAULT: was 'ente alerts'`: Already rebranded; the hit is the PHOTOVAULT comment recording the old value.
  - L402 `'Photovault alerts', // PHOTOVAULT: was 'ente alerts'`: Already rebranded; the hit is the PHOTOVAULT comment recording the old value.
- `apps/photos/lib/services/push_service.dart` (`ente (word)`×1, `firebase`×1): only comments; internal.
- `apps/photos/lib/services/remote_assets_service.dart` (`ente.com`×1, `ente (word)`×1):
  - L616 `return host == "models.ente.com";`: Host check that only decides extra download logging.
- `apps/photos/lib/services/rituals/rituals_service.dart` (`ente (word)`×1, `subscription`×3):
  - L535 `scheme: "ente",`: Internal `ente://` deep-link scheme (passkey redirect, notification routing); never shown.
  - remaining hits are identifiers (class/field/variable names such as `StreamSubscription`, `EnteFile`, `Subscription` model); internal.
- `apps/photos/lib/services/search_service.dart` (`subscription`×3): only identifiers (class/field/variable names such as `StreamSubscription`, `EnteFile`, `Subscription` model); internal.
- `apps/photos/lib/services/social_notification_coordinator.dart` (`ente (word)`×2):
  - L291 `channelName: 'Feed', // PHOTOVAULT: branding (was 'Ente Feed')`: Already rebranded; the hit is the PHOTOVAULT comment recording the old value.
  - L534 `scheme: 'ente',`: Internal `ente://` deep-link scheme (passkey redirect, notification routing); never shown.
- `apps/photos/lib/services/storage_bonus_service.dart` (`referral`×3): only identifiers (class/field/variable names such as `StreamSubscription`, `EnteFile`, `Subscription` model); internal.
- `apps/photos/lib/services/sync/local_sync_service.dart` (`subscription`×4): only identifiers (class/field/variable names such as `StreamSubscription`, `EnteFile`, `Subscription` model); internal.
- `apps/photos/lib/services/sync/remote_sync_service.dart` (`ente (word)`×2, `subscription`×4):
  - L1149 `payload: "ente://collection/?collectionID=$collectionID",`: Internal `ente://` deep-link scheme (passkey redirect, notification routing); never shown.
  - remaining hits are comments, identifiers (class/field/variable names such as `StreamSubscription`, `EnteFile`, `Subscription` model); internal.
- `apps/photos/lib/services/sync/sync_service.dart` (`subscription`×4): only package imports (`ente_*` packages), identifiers (class/field/variable names such as `StreamSubscription`, `EnteFile`, `Subscription` model); internal.
- `apps/photos/lib/services/update_service.dart` (`ente.com`×1, `ente (word)`×6):
  - L159 `return _packageInfo.packageName.startsWith("io.ente.photos.independent");`: Platform identifier (namespace, bundle id, channel, task or app-group id); internal.
  - L166 `return _packageInfo.packageName.startsWith("io.ente.photos.fdroid");`: Platform identifier (namespace, bundle id, channel, task or app-group id); internal.
  - remaining hits are comments; internal.
- `apps/photos/lib/states/all_sections_examples_state.dart` (`subscription`×4): only identifiers (class/field/variable names such as `StreamSubscription`, `EnteFile`, `Subscription` model); internal.
- `apps/photos/lib/states/location_screen_state.dart` (`subscription`×1): only identifiers (class/field/variable names such as `StreamSubscription`, `EnteFile`, `Subscription` model); internal.
- `apps/photos/lib/states/location_state.dart` (`subscription`×1): only identifiers (class/field/variable names such as `StreamSubscription`, `EnteFile`, `Subscription` model); internal.
- `apps/photos/lib/states/user_details_state.dart` (`subscription`×7): only identifiers (class/field/variable names such as `StreamSubscription`, `EnteFile`, `Subscription` model); internal.
- `apps/photos/lib/theme/colors.dart` (`ente (word)`×2): only comments; internal.
- `apps/photos/lib/ui/account/email_entry_page.dart` (`ente.com`×1, `ente (word)`×2, `referral`×18): correction to the first version of this report: one `referral` hit was user-visible, the "How did you hear about Photovault?" field (`hearUsWhereTitle`, Ente's growth attribution). It is now hidden (`_showReferralSourceField` returns false, PHOTOVAULT comment). The remaining hits are comments and identifiers (`showReferralSourceField`, `referralSource`, `setRefSource`, which still sends the route source or an empty string); internal.
- `apps/photos/lib/ui/account/login_pwd_verification_page.dart` (`ente.com`×1, `ente (word)`×1):
  - L245 `"support@ente.com",`: `toEmail` argument that Photos' `sendLogs` ignores (logs are only exported/shared).
- `apps/photos/lib/ui/account/onboarding_page_scaffold.dart` (`ente (word)`×1, `duck`×2): only comments; internal.
- `apps/photos/lib/ui/account/passkey_page.dart` (`ente (word)`×3, `subscription`×3):
  - L60 `"&redirect=ente://passkey"`: Internal `ente://` deep-link scheme (passkey redirect, notification routing); never shown.
  - L112 `if (mounted && link.toLowerCase().startsWith("ente://passkey")) {`: Internal `ente://` deep-link scheme (passkey redirect, notification routing); never shown.
  - L61 `"&clientPackage=io.ente.photos",`: Platform identifier (namespace, bundle id, channel, task or app-group id); internal.
  - remaining hits are identifiers (class/field/variable names such as `StreamSubscription`, `EnteFile`, `Subscription` model); internal.
- `apps/photos/lib/ui/account/password_entry_page.dart` (`ente.com`×1, `ente (word)`×2, `subscription`×3): only comments, package imports (`ente_*` packages), identifiers (class/field/variable names such as `StreamSubscription`, `EnteFile`, `Subscription` model); internal.
- `apps/photos/lib/ui/account/password_reentry_page.dart` (`ente.com`×1, `ente (word)`×1, `subscription`×2):
  - L180 `"support@ente.com",`: `toEmail` argument that Photos' `sendLogs` ignores (logs are only exported/shared).
  - remaining hits are package imports (`ente_*` packages), identifiers (class/field/variable names such as `StreamSubscription`, `EnteFile`, `Subscription` model); internal.
- `apps/photos/lib/ui/account/two_factor_recovery_page.dart` (`ente (word)`×1): only comments; internal.
- `apps/photos/lib/ui/actions/collection/collection_file_actions.dart` (`ente (word)`×2): only comments; internal.
- `apps/photos/lib/ui/collections/album/horizontal_list.dart` (`subscription`×4): only identifiers (class/field/variable names such as `StreamSubscription`, `EnteFile`, `Subscription` model); internal.
- `apps/photos/lib/ui/collections/collection_action_sheet.dart` (`subscription`×3): only identifiers (class/field/variable names such as `StreamSubscription`, `EnteFile`, `Subscription` model); internal.
- `apps/photos/lib/ui/collections/collection_list_page.dart` (`subscription`×7): only identifiers (class/field/variable names such as `StreamSubscription`, `EnteFile`, `Subscription` model); internal.
- `apps/photos/lib/ui/collections/device/device_folders_vertical_grid_view.dart` (`subscription`×4): only identifiers (class/field/variable names such as `StreamSubscription`, `EnteFile`, `Subscription` model); internal.
- `apps/photos/lib/ui/collections/flex_grid_view.dart` (`subscription`×4): only identifiers (class/field/variable names such as `StreamSubscription`, `EnteFile`, `Subscription` model); internal.
- `apps/photos/lib/ui/common/photo_library_add_permission.dart` (`duck`×1):
  - L40 `assetPath: "assets/ducky_smart_feature.png",`: Asset file name kept so code is untouched; the image was re-rendered without the mascot.
- `apps/photos/lib/ui/components/banners/ml_progress_banner.dart` (`subscription`×7): only identifiers (class/field/variable names such as `StreamSubscription`, `EnteFile`, `Subscription` model); internal.
- `apps/photos/lib/ui/components/banners/name_face_banner.dart` (`referral`×2, `subscription`×1): only identifiers (class/field/variable names such as `StreamSubscription`, `EnteFile`, `Subscription` model); internal.
- `apps/photos/lib/ui/components/banners/save_faces_banner.dart` (`referral`×2, `subscription`×1): only identifiers (class/field/variable names such as `StreamSubscription`, `EnteFile`, `Subscription` model); internal.
- `apps/photos/lib/ui/components/bottom_of_title_bar_widget.dart` (`ente (word)`×1): only comments; internal.
- `apps/photos/lib/ui/components/empty_state_illustration.dart` (`ente (word)`×1, `duck`×1): only comments; internal.
- `apps/photos/lib/ui/components/empty_state_item_widget.dart` (`ente (word)`×1): only comments; internal.
- `apps/photos/lib/ui/components/info_item_widget.dart` (`ente (word)`×1): only comments; internal.
- `apps/photos/lib/ui/family/family_plan_page.dart` (`ente (word)`×1, `subscription`×9): only comments, identifiers (class/field/variable names such as `StreamSubscription`, `EnteFile`, `Subscription` model); internal.
- `apps/photos/lib/ui/home/christmas/christmas_utils.dart` (`ente (word)`×1): only comments; internal.
- `apps/photos/lib/ui/home/gallery_download_banner.dart` (`subscription`×9): only identifiers (class/field/variable names such as `StreamSubscription`, `EnteFile`, `Subscription` model); internal.
- `apps/photos/lib/ui/home/grant_permissions_widget.dart` (`ente (word)`×6, `duck`×2):
  - L96 `"assets/ente-branding.svg",`: Asset file name kept; the file contains the Photovault wordmark.
  - L512 `"assets/ente-branding.svg",`: Asset file name kept; the file contains the Photovault wordmark.
  - L344 `assetPath: 'assets/ducky_smart_feature.png',`: Asset file name kept so code is untouched; the image was re-rendered without the mascot.
  - remaining hits are comments; internal.
- `apps/photos/lib/ui/home/header_error_widget.dart` (`ente.com`×1, `ente (word)`×2, `subscription`×1):
  - L70 `"support@ente.com",`: `toEmail` argument that Photos' `sendLogs` ignores (logs are only exported/shared).
  - remaining hits are comments; internal.
- `apps/photos/lib/ui/home/header_widget.dart` (`ente (word)`×1, `subscription`×1): only comments, identifiers (class/field/variable names such as `StreamSubscription`, `EnteFile`, `Subscription` model); internal.
- `apps/photos/lib/ui/home/home_bottom_nav_bar.dart` (`ente (word)`×4, `subscription`×3):
  - L165 `semanticIdentifier: 'ente.photos.tab.home',`: Widget key / restoration id; internal.
  - L171 `semanticIdentifier: 'ente.photos.tab.albums',`: Widget key / restoration id; internal.
  - L177 `semanticIdentifier: 'ente.photos.tab.feed',`: Widget key / restoration id; internal.
  - L183 `semanticIdentifier: 'ente.photos.tab.search',`: Widget key / restoration id; internal.
  - remaining hits are identifiers (class/field/variable names such as `StreamSubscription`, `EnteFile`, `Subscription` model); internal.
- `apps/photos/lib/ui/home/home_gallery_widget.dart` (`subscription`×4): only identifiers (class/field/variable names such as `StreamSubscription`, `EnteFile`, `Subscription` model); internal.
- `apps/photos/lib/ui/home/landing_page_widget.dart` (`ente (word)`×1, `duck`×1, `subscription`×3): only comments, package imports (`ente_*` packages), identifiers (class/field/variable names such as `StreamSubscription`, `EnteFile`, `Subscription` model); internal.
- `apps/photos/lib/ui/home/large_backup_screen.dart` (`duck`×1):
  - L67 `"assets/backup_mode_ducky.png",`: Asset file name kept so code is untouched; the image was re-rendered without the mascot.
- `apps/photos/lib/ui/home/loading_photos_widget.dart` (`ente.com`×2, `ente (word)`×3, `subscription`×3):
  - L262 `"support@ente.com",`: `toEmail` argument that Photos' `sendLogs` ignores (logs are only exported/shared).
  - remaining hits are comments, identifiers (class/field/variable names such as `StreamSubscription`, `EnteFile`, `Subscription` model); internal.
- `apps/photos/lib/ui/home/memories/full_screen_memory.dart` (`subscription`×11): only identifiers (class/field/variable names such as `StreamSubscription`, `EnteFile`, `Subscription` model); internal.
- `apps/photos/lib/ui/home/memories/memories_strip.dart` (`subscription`×26): only identifiers (class/field/variable names such as `StreamSubscription`, `EnteFile`, `Subscription` model); internal.
- `apps/photos/lib/ui/home/memories/memory_share_sheet.dart` (`ente (word)`×1): only comments; internal.
- `apps/photos/lib/ui/home/status_bar_widget.dart` (`ente (word)`×2, `subscription`×12):
  - L137 `"assets/ente-branding.svg",`: Asset file name kept; the file contains the Photovault wordmark.
  - remaining hits are comments, identifiers (class/field/variable names such as `StreamSubscription`, `EnteFile`, `Subscription` model); internal.
- `apps/photos/lib/ui/map/tile/attribution/map_attribution.dart` (`subscription`×4): only identifiers (class/field/variable names such as `StreamSubscription`, `EnteFile`, `Subscription` model); internal.
- `apps/photos/lib/ui/map/tile/layers.dart` (`ente (word)`×1): only comments; internal.
- `apps/photos/lib/ui/photovault/photovault_glyph.dart` (`ente (word)`×1, `duck`×1, `mascot`×2): only comments; internal.
- `apps/photos/lib/ui/photovault/photovault_settings_footer.dart` (`ente (word)`×2): only comments; internal.
- `apps/photos/lib/ui/rituals/ritual_share_card.dart` (`duck`×8):
  - L7 `const String _duckyShareArtAsset = "assets/rituals/ritual_ducky_share.svg";`: Asset file name kept so code is untouched; the image was re-rendered without the mascot.
  - remaining hits are identifiers (class/field/variable names such as `StreamSubscription`, `EnteFile`, `Subscription` model); internal.
- `apps/photos/lib/ui/settings/about/about_us_page.dart` (`ente (word)`×3): only comments; internal.
- `apps/photos/lib/ui/settings/account/account_settings_page.dart` (`ente (word)`×1, `subscription`×1): only comments; internal.
- `apps/photos/lib/ui/settings/app_icon_selection_screen.dart` (`duck`×1, `mascot`×1): only comments; internal.
- `apps/photos/lib/ui/settings/backup/backup_item_card.dart` (`ente.com`×1, `ente (word)`×1):
  - L157 `"support@ente.com",`: `toEmail` argument that Photos' `sendLogs` ignores (logs are only exported/shared).
- `apps/photos/lib/ui/settings/backup/backup_status_screen.dart` (`subscription`×6): only identifiers (class/field/variable names such as `StreamSubscription`, `EnteFile`, `Subscription` model); internal.
- `apps/photos/lib/ui/settings/backup/free_space_options.dart` (`ente (word)`×1, `rate us`×2): only comments; internal.
- `apps/photos/lib/ui/settings/debug/ml_debug_settings_page.dart` (`ente (word)`×1):
  - L592 `title: "Recompute Rewind", // PHOTOVAULT: was "Ente Rewind"`: Already rebranded; the hit is the PHOTOVAULT comment recording the old value.
- `apps/photos/lib/ui/settings/ml/machine_learning_settings_page.dart` (`ente.com`×1, `ente (word)`×1, `duck`×1, `subscription`×1):
  - L622 `"support@ente.com",`: `toEmail` argument that Photos' `sendLogs` ignores (logs are only exported/shared).
  - L173 `"assets/ducky_ml.png",`: Asset file name kept so code is untouched; the image was re-rendered without the mascot.
  - remaining hits are identifiers (class/field/variable names such as `StreamSubscription`, `EnteFile`, `Subscription` model); internal.
- `apps/photos/lib/ui/settings/ml/ml_consent_sheet.dart` (`ente (word)`×1, `duck`×1):
  - L101 `"assets/ducky_ml.png",`: Asset file name kept so code is untouched; the image was re-rendered without the mascot.
  - remaining hits are comments; internal.
- `apps/photos/lib/ui/settings/search/settings_search_registry.dart` (`ente (word)`×3, `subscription`×1): only comments; internal.
- `apps/photos/lib/ui/settings/security/security_settings_page.dart` (`subscription`×1): only identifiers (class/field/variable names such as `StreamSubscription`, `EnteFile`, `Subscription` model); internal.
- `apps/photos/lib/ui/settings/storage_card_widget.dart` (`ente (word)`×1): only comments; internal.
- `apps/photos/lib/ui/settings/streaming/video_streaming_settings_page.dart` (`ente (word)`×2, `subscription`×3): only comments, identifiers (class/field/variable names such as `StreamSubscription`, `EnteFile`, `Subscription` model); internal.
- `apps/photos/lib/ui/settings_page.dart` (`ente (word)`×4, `referral`×1, `merch`×1, `rate us`×1): only comments; internal.
- `apps/photos/lib/ui/sharing/library_sharing/library_sharing_page.dart` (`ente (word)`×1, `duck`×2):
  - L203 `? 'assets/ducky_full_library.png'`: Asset file name kept so code is untouched; the image was re-rendered without the mascot.
  - L204 `: 'assets/ducky_share.png',`: Asset file name kept so code is untouched; the image was re-rendered without the mascot.
  - remaining hits are comments; internal.
- `apps/photos/lib/ui/sharing/public_link_enabled_actions_widget.dart` (`ente (word)`×1):
  - L111 `assetPath: 'assets/ente-branding.svg',`: Asset file name kept; the file contains the Photovault wordmark.
- `apps/photos/lib/ui/sharing/share_collection_page.dart` (`subscription`×4): only identifiers (class/field/variable names such as `StreamSubscription`, `EnteFile`, `Subscription` model); internal.
- `apps/photos/lib/ui/sharing/user_avatar_widget.dart` (`subscription`×6): only identifiers (class/field/variable names such as `StreamSubscription`, `EnteFile`, `Subscription` model); internal.
- `apps/photos/lib/ui/sharing/widgets/sharing_progress_sheet.dart` (`ente (word)`×1, `duck`×1): only comments; internal.
- `apps/photos/lib/ui/social/feed_screen.dart` (`subscription`×3): only identifiers (class/field/variable names such as `StreamSubscription`, `EnteFile`, `Subscription` model); internal.
- `apps/photos/lib/ui/social/widgets/comment_bubble_widget.dart` (`subscription`×4): only identifiers (class/field/variable names such as `StreamSubscription`, `EnteFile`, `Subscription` model); internal.
- `apps/photos/lib/ui/social/widgets/delete_comment_confirmation_dialog.dart` (`duck`×1):
  - L61 `"assets/ducky_garbage_bin_opened.png",`: Asset file name kept so code is untouched; the image was re-rendered without the mascot.
- `apps/photos/lib/ui/social/widgets/file_social_overlay.dart` (`subscription`×4): only identifiers (class/field/variable names such as `StreamSubscription`, `EnteFile`, `Subscription` model); internal.
- `apps/photos/lib/ui/social/widgets/resolved_social_user_name.dart` (`subscription`×6): only identifiers (class/field/variable names such as `StreamSubscription`, `EnteFile`, `Subscription` model); internal.
- `apps/photos/lib/ui/tabs/albums_tab.dart` (`ente (word)`×11, `duck`×1, `subscription`×18):
  - L542 `assetPath: "assets/ente-nomatch-ducky.riv",`: Asset path no longer loaded: `EmptyStateIllustration` draws the Photovault glyph.
  - remaining hits are identifiers (class/field/variable names such as `StreamSubscription`, `EnteFile`, `Subscription` model); internal.
- `apps/photos/lib/ui/tabs/home_widget.dart` (`ente.io`×1, `ente.com`×2, `ente (word)`×7, `subscription`×41):
  - L841 `return host == "albums.ente.io" || host == "albums.ente.com";`: Recognises Ente public-album link hosts (internal host check).
  - L319 `if (packageName == 'io.ente.photos.independent' ||`: Platform identifier (namespace, bundle id, channel, task or app-group id); internal.
  - L320 `packageName == 'io.ente.photos.fdroid') {`: Platform identifier (namespace, bundle id, channel, task or app-group id); internal.
  - L830 `return uri.scheme == "ente" || uri.scheme == "https";`: Internal `ente://` deep-link scheme (passkey redirect, notification routing); never shown.
  - L795 `"No initial link received in public album link subscription.",`: Log message ("stream subscription"); not shown.
  - L813 `_logger.info("No link received in public album link subscription.");`: Log message ("stream subscription"); not shown.
  - remaining hits are comments, package imports (`ente_*` packages), identifiers (class/field/variable names such as `StreamSubscription`, `EnteFile`, `Subscription` model); internal.
- `apps/photos/lib/ui/tabs/shared/all_links_page.dart` (`ente (word)`×1, `duck`×1, `subscription`×4):
  - L284 `assetPath: "assets/ente-links-ducky.riv",`: Asset path no longer loaded: `EmptyStateIllustration` draws the Photovault glyph.
  - remaining hits are identifiers (class/field/variable names such as `StreamSubscription`, `EnteFile`, `Subscription` model); internal.
- `apps/photos/lib/ui/tools/editor/image_editor/image_editor_page.dart` (`ente (word)`×3): only package imports (`ente_*` packages), identifiers (class/field/variable names such as `StreamSubscription`, `EnteFile`, `Subscription` model); internal.
- `apps/photos/lib/ui/tools/editor/video_editor/video_trim_slider.dart` (`ente (word)`×1):
  - L259 `'ente-video-timeline-',`: Internal temp-file / key prefix; not shown.
- `apps/photos/lib/ui/tools/editor/video_editor_page.dart` (`ente (word)`×2):
  - L336 `outputPrefix: 'ente-native-export',`: Internal temp-file / key prefix; not shown.
  - L360 `outputPrefix: 'ente-ffmpeg-export',`: Internal temp-file / key prefix; not shown.
- `apps/photos/lib/ui/tools/photo_review/review_month_page.dart` (`subscription`×3): only identifiers (class/field/variable names such as `StreamSubscription`, `EnteFile`, `Subscription` model); internal.
- `apps/photos/lib/ui/tools/photo_review/review_old_photos_page.dart` (`subscription`×6): only identifiers (class/field/variable names such as `StreamSubscription`, `EnteFile`, `Subscription` model); internal.
- `apps/photos/lib/ui/tools/similar_images_page.dart` (`ente (word)`×1, `duck`×3):
  - L348 `"assets/ducky_cleaning_static.svg",`: Asset file name kept so code is untouched; the image was re-rendered without the mascot.
  - L1162 `SvgPicture.asset("assets/ducky_cleaning_static.svg", height: 160),`: Asset file name kept so code is untouched; the image was re-rendered without the mascot.
  - remaining hits are comments; internal.
- `apps/photos/lib/ui/viewer/file/detail_page.dart` (`subscription`×7): only identifiers (class/field/variable names such as `StreamSubscription`, `EnteFile`, `Subscription` model); internal.
- `apps/photos/lib/ui/viewer/file/external_video_viewer.dart` (`subscription`×4): only identifiers (class/field/variable names such as `StreamSubscription`, `EnteFile`, `Subscription` model); internal.
- `apps/photos/lib/ui/viewer/file/file_app_bar.dart` (`subscription`×3): only identifiers (class/field/variable names such as `StreamSubscription`, `EnteFile`, `Subscription` model); internal.
- `apps/photos/lib/ui/viewer/file/file_bottom_bar.dart` (`subscription`×3): only identifiers (class/field/variable names such as `StreamSubscription`, `EnteFile`, `Subscription` model); internal.
- `apps/photos/lib/ui/viewer/file/file_details_widget.dart` (`subscription`×1): only identifiers (class/field/variable names such as `StreamSubscription`, `EnteFile`, `Subscription` model); internal.
- `apps/photos/lib/ui/viewer/file/native_video_player_controls/play_pause_button.dart` (`subscription`×3): only identifiers (class/field/variable names such as `StreamSubscription`, `EnteFile`, `Subscription` model); internal.
- `apps/photos/lib/ui/viewer/file/native_video_player_controls/seek_bar.dart` (`subscription`×6): only identifiers (class/field/variable names such as `StreamSubscription`, `EnteFile`, `Subscription` model); internal.
- `apps/photos/lib/ui/viewer/file/video_control/mute_button.dart` (`subscription`×3): only identifiers (class/field/variable names such as `StreamSubscription`, `EnteFile`, `Subscription` model); internal.
- `apps/photos/lib/ui/viewer/file/video_widget.dart` (`subscription`×10): only identifiers (class/field/variable names such as `StreamSubscription`, `EnteFile`, `Subscription` model); internal.
- `apps/photos/lib/ui/viewer/file/video_widget_media_kit.dart` (`subscription`×20): only identifiers (class/field/variable names such as `StreamSubscription`, `EnteFile`, `Subscription` model); internal.
- `apps/photos/lib/ui/viewer/file/video_widget_media_kit_common.dart` (`subscription`×15): only identifiers (class/field/variable names such as `StreamSubscription`, `EnteFile`, `Subscription` model); internal.
- `apps/photos/lib/ui/viewer/file/video_widget_native.dart` (`subscription`×24): only identifiers (class/field/variable names such as `StreamSubscription`, `EnteFile`, `Subscription` model); internal.
- `apps/photos/lib/ui/viewer/file/zoomable_image.dart` (`subscription`×7): only identifiers (class/field/variable names such as `StreamSubscription`, `EnteFile`, `Subscription` model); internal.
- `apps/photos/lib/ui/viewer/file/zoomable_live_image_new.dart` (`subscription`×3): only identifiers (class/field/variable names such as `StreamSubscription`, `EnteFile`, `Subscription` model); internal.
- `apps/photos/lib/ui/viewer/file_details/favorite_widget.dart` (`subscription`×4): only identifiers (class/field/variable names such as `StreamSubscription`, `EnteFile`, `Subscription` model); internal.
- `apps/photos/lib/ui/viewer/file_details/file_info_faces_item_widget.dart` (`subscription`×1): only identifiers (class/field/variable names such as `StreamSubscription`, `EnteFile`, `Subscription` model); internal.
- `apps/photos/lib/ui/viewer/file_details/location_tags_widget.dart` (`subscription`×1): only identifiers (class/field/variable names such as `StreamSubscription`, `EnteFile`, `Subscription` model); internal.
- `apps/photos/lib/ui/viewer/file_details/upload_icon_widget.dart` (`subscription`×1): only identifiers (class/field/variable names such as `StreamSubscription`, `EnteFile`, `Subscription` model); internal.
- `apps/photos/lib/ui/viewer/gallery/archive_page.dart` (`ente (word)`×1, `duck`×1, `subscription`×7):
  - L144 `assetPath: "assets/ente-archive-ducky.riv",`: Asset path no longer loaded: `EmptyStateIllustration` draws the Photovault glyph.
  - remaining hits are identifiers (class/field/variable names such as `StreamSubscription`, `EnteFile`, `Subscription` model); internal.
- `apps/photos/lib/ui/viewer/gallery/collection_page.dart` (`subscription`×4): only identifiers (class/field/variable names such as `StreamSubscription`, `EnteFile`, `Subscription` model); internal.
- `apps/photos/lib/ui/viewer/gallery/component/gallery_file_widget.dart` (`ente (word)`×1, `subscription`×3): only comments, identifiers (class/field/variable names such as `StreamSubscription`, `EnteFile`, `Subscription` model); internal.
- `apps/photos/lib/ui/viewer/gallery/device/backup_header_widget.dart` (`subscription`×1): only identifiers (class/field/variable names such as `StreamSubscription`, `EnteFile`, `Subscription` model); internal.
- `apps/photos/lib/ui/viewer/gallery/device/skipped_device_folder_page.dart` (`subscription`×1): only identifiers (class/field/variable names such as `StreamSubscription`, `EnteFile`, `Subscription` model); internal.
- `apps/photos/lib/ui/viewer/gallery/gallery.dart` (`ente (word)`×1, `subscription`×11):
  - L998 `return "ente.photos.gallery.$safeTagPrefix.scroll";`: Widget key / restoration id; internal.
  - remaining hits are identifiers (class/field/variable names such as `StreamSubscription`, `EnteFile`, `Subscription` model); internal.
- `apps/photos/lib/ui/viewer/gallery/gallery_app_bar_widget.dart` (`ente.com`×1, `ente (word)`×2, `rate us`×1, `subscription`×8): only comments, package imports (`ente_*` packages), identifiers (class/field/variable names such as `StreamSubscription`, `EnteFile`, `Subscription` model); internal.
- `apps/photos/lib/ui/viewer/gallery/hidden_page.dart` (`subscription`×5): only identifiers (class/field/variable names such as `StreamSubscription`, `EnteFile`, `Subscription` model); internal.
- `apps/photos/lib/ui/viewer/gallery/hierarchical_search_gallery.dart` (`subscription`×1): only identifiers (class/field/variable names such as `StreamSubscription`, `EnteFile`, `Subscription` model); internal.
- `apps/photos/lib/ui/viewer/gallery/jump_to_date_gallery.dart` (`subscription`×1): only identifiers (class/field/variable names such as `StreamSubscription`, `EnteFile`, `Subscription` model); internal.
- `apps/photos/lib/ui/viewer/gallery/trash_page.dart` (`ente (word)`×2, `duck`×1):
  - L139 `assetPath: "assets/ente-trash-ducky.riv",`: Asset path no longer loaded: `EmptyStateIllustration` draws the Photovault glyph.
  - remaining hits are identifiers (class/field/variable names such as `StreamSubscription`, `EnteFile`, `Subscription` model); internal.
- `apps/photos/lib/ui/viewer/location/location_screen.dart` (`subscription`×1): only identifiers (class/field/variable names such as `StreamSubscription`, `EnteFile`, `Subscription` model); internal.
- `apps/photos/lib/ui/viewer/people/cluster_app_bar.dart` (`subscription`×5): only package imports (`ente_*` packages), identifiers (class/field/variable names such as `StreamSubscription`, `EnteFile`, `Subscription` model); internal.
- `apps/photos/lib/ui/viewer/people/cluster_page.dart` (`subscription`×3): only identifiers (class/field/variable names such as `StreamSubscription`, `EnteFile`, `Subscription` model); internal.
- `apps/photos/lib/ui/viewer/people/memory_lane_page_v2.dart` (`ente (word)`×2): only comments; internal.
- `apps/photos/lib/ui/viewer/people/people_app_bar.dart` (`subscription`×8): only package imports (`ente_*` packages), identifiers (class/field/variable names such as `StreamSubscription`, `EnteFile`, `Subscription` model); internal.
- `apps/photos/lib/ui/viewer/people/people_page.dart` (`subscription`×2): only identifiers (class/field/variable names such as `StreamSubscription`, `EnteFile`, `Subscription` model); internal.
- `apps/photos/lib/ui/viewer/people/person_cluster_suggestion.dart` (`subscription`×2): only package imports (`ente_*` packages), identifiers (class/field/variable names such as `StreamSubscription`, `EnteFile`, `Subscription` model); internal.
- `apps/photos/lib/ui/viewer/people/person_face_widget.dart` (`subscription`×6): only identifiers (class/field/variable names such as `StreamSubscription`, `EnteFile`, `Subscription` model); internal.
- `apps/photos/lib/ui/viewer/people/person_gallery_suggestion.dart` (`subscription`×1): only identifiers (class/field/variable names such as `StreamSubscription`, `EnteFile`, `Subscription` model); internal.
- `apps/photos/lib/ui/viewer/search/contact_avatar_widget.dart` (`subscription`×3): only identifiers (class/field/variable names such as `StreamSubscription`, `EnteFile`, `Subscription` model); internal.
- `apps/photos/lib/ui/viewer/search/result/contact_result_page.dart` (`duck`×1, `subscription`×2):
  - L538 `"assets/ducky_share.png",`: Asset file name kept so code is untouched; the image was re-rendered without the mascot.
  - remaining hits are identifiers (class/field/variable names such as `StreamSubscription`, `EnteFile`, `Subscription` model); internal.
- `apps/photos/lib/ui/viewer/search/result/magic_result_screen.dart` (`subscription`×2): only identifiers (class/field/variable names such as `StreamSubscription`, `EnteFile`, `Subscription` model); internal.
- `apps/photos/lib/ui/viewer/search/result/people_section_all_page.dart` (`subscription`×4): only identifiers (class/field/variable names such as `StreamSubscription`, `EnteFile`, `Subscription` model); internal.
- `apps/photos/lib/ui/viewer/search/result/search_result_page.dart` (`subscription`×1): only identifiers (class/field/variable names such as `StreamSubscription`, `EnteFile`, `Subscription` model); internal.
- `apps/photos/lib/ui/viewer/search/result/search_section_all_page.dart` (`subscription`×4): only identifiers (class/field/variable names such as `StreamSubscription`, `EnteFile`, `Subscription` model); internal.
- `apps/photos/lib/ui/viewer/search/search_suggestions.dart` (`subscription`×3): only identifiers (class/field/variable names such as `StreamSubscription`, `EnteFile`, `Subscription` model); internal.
- `apps/photos/lib/ui/viewer/search/search_widget.dart` (`subscription`×3): only identifiers (class/field/variable names such as `StreamSubscription`, `EnteFile`, `Subscription` model); internal.
- `apps/photos/lib/ui/viewer/search_tab/contacts_section.dart` (`subscription`×4): only identifiers (class/field/variable names such as `StreamSubscription`, `EnteFile`, `Subscription` model); internal.
- `apps/photos/lib/ui/viewer/search_tab/locations_section.dart` (`subscription`×4): only identifiers (class/field/variable names such as `StreamSubscription`, `EnteFile`, `Subscription` model); internal.
- `apps/photos/lib/ui/viewer/search_tab/magic_section.dart` (`subscription`×4): only identifiers (class/field/variable names such as `StreamSubscription`, `EnteFile`, `Subscription` model); internal.
- `apps/photos/lib/ui/viewer/search_tab/people_section.dart` (`subscription`×4): only identifiers (class/field/variable names such as `StreamSubscription`, `EnteFile`, `Subscription` model); internal.
- `apps/photos/lib/ui/wrapped/cards/badge_card_content.dart` (`duck`×24): only comments, identifiers (class/field/variable names such as `StreamSubscription`, `EnteFile`, `Subscription` model); internal.
- `apps/photos/lib/ui/wrapped/rewind_banner.dart` (`ente (word)`×1): only comments; internal.
- `apps/photos/lib/ui/wrapped/wrapped_discovery_section.dart` (`ente (word)`×1): only comments; internal.
- `apps/photos/lib/ui/wrapped/wrapped_rewind_banner_button.dart` (`ente (word)`×1): only comments; internal.
- `apps/photos/lib/ui/wrapped/wrapped_viewer_page.dart` (`ente (word)`×9, `subscription`×3):
  - L970 `_logger.severe("Failed to share Ente Rewind card", error, stackTrace);`: Log/exception message for developers; not shown in UI.
  - L1216 `"Failed to initialize Ente Rewind music",`: Log/exception message for developers; not shown in UI.
  - L1256 `"Failed to restart Ente Rewind music",`: Log/exception message for developers; not shown in UI.
  - L1281 `_logger.fine("Failed to pause Ente Rewind music", error, stackTrace);`: Log/exception message for developers; not shown in UI.
  - L1302 `"Failed to play Ente Rewind music",`: Log/exception message for developers; not shown in UI.
  - L1311 `_logger.warning("Failed to play Ente Rewind music", error, stackTrace);`: Log/exception message for developers; not shown in UI.
  - L1364 `_logger.fine("Failed to fade out Ente Rewind music", error, stackTrace);`: Log/exception message for developers; not shown in UI.
  - L1371 `"Failed to restore Ente Rewind music volume",`: Log/exception message for developers; not shown in UI.
  - L1455 `"Failed to configure audio session for Ente Rewind music",`: Log/exception message for developers; not shown in UI.
  - remaining hits are identifiers (class/field/variable names such as `StreamSubscription`, `EnteFile`, `Subscription` model); internal.
- `apps/photos/lib/utils/background_tasks.dart` (`ente (word)`×2, `sentry`×1):
  - L30 `static const refresh = "io.ente.photos.nativeBackgroundRefresh";`: Platform identifier (namespace, bundle id, channel, task or app-group id); internal.
  - L31 `static const processing = "io.ente.photos.nativeBackgroundProcessing";`: Platform identifier (namespace, bundle id, channel, task or app-group id); internal.
  - remaining hits are identifiers (class/field/variable names such as `StreamSubscription`, `EnteFile`, `Subscription` model); internal.
- `apps/photos/lib/utils/bg_task_utils.dart` (`ente (word)`×5, `sentry`×1):
  - L135 `"io.ente.frame.iOSBackgroundAppRefresh";`: Platform identifier (namespace, bundle id, channel, task or app-group id); internal.
  - L137 `"io.ente.frame.iOSBackgroundProcessing";`: Platform identifier (namespace, bundle id, channel, task or app-group id); internal.
  - L138 `static const androidPeriodicTask = "io.ente.photos.androidPeriodicTask";`: Platform identifier (namespace, bundle id, channel, task or app-group id); internal.
  - L139 `static const androidPushRefreshTask = "io.ente.photos.androidPushRefresh";`: Platform identifier (namespace, bundle id, channel, task or app-group id); internal.
  - L141 `"io.ente.photos.androidBackgroundProcessing";`: Platform identifier (namespace, bundle id, channel, task or app-group id); internal.
  - remaining hits are identifiers (class/field/variable names such as `StreamSubscription`, `EnteFile`, `Subscription` model); internal.
- `apps/photos/lib/utils/collection_util.dart` (`ente (word)`×6):
  - L25 `if (packageName == 'io.ente.photos.independent') {`: Platform identifier (namespace, bundle id, channel, task or app-group id); internal.
  - L28 `package: 'io.ente.photos.independent',`: Platform identifier (namespace, bundle id, channel, task or app-group id); internal.
  - L29 `data: 'package:io.ente.photos.independent',`: Platform identifier (namespace, bundle id, channel, task or app-group id); internal.
  - L32 `} else if (packageName == 'io.ente.photos.fdroid') {`: Platform identifier (namespace, bundle id, channel, task or app-group id); internal.
  - L35 `package: 'io.ente.photos.fdroid',`: Platform identifier (namespace, bundle id, channel, task or app-group id); internal.
  - L36 `data: 'package:io.ente.photos.fdroid',`: Platform identifier (namespace, bundle id, channel, task or app-group id); internal.
- `apps/photos/lib/utils/delete_file_util.dart` (`ente (word)`×1, `duck`×1):
  - L355 `final trashKind = isOnEnteTrash ? "Ente" : "device";`: Log/exception message for developers; not shown in UI.
  - L837 `illustration: Image.asset("assets/ducky_smart_feature.png"),`: Asset file name kept so code is untouched; the image was re-rendered without the mascot.
- `apps/photos/lib/utils/dialog_util.dart` (`ente.com`×2, `ente (word)`×2):
  - L206 `"support@ente.com",`: `toEmail` argument that Photos' `sendLogs` ignores (logs are only exported/shared).
  - L267 `"support@ente.com",`: `toEmail` argument that Photos' `sendLogs` ignores (logs are only exported/shared).
- `apps/photos/lib/utils/email_util.dart` (`ente (word)`×3): only comments; internal.
- `apps/photos/lib/utils/share_util.dart` (`ente (word)`×1): only comments; internal.
- `apps/photos/pubspec.yaml` (`ente (word)`×7, `mascot`×1, `subscription`×1, `sentry`×1, `firebase`×2): Package description, github.com/ente plugin forks used as dependency sources, PHOTOVAULT comments; not shown.

## Shared packages (`mobile/packages`)

- `packages/account_deletion/lib/src/ui/delete_account_confirmation_step.dart` (`ente (word)`×1): only comments; internal.
- `packages/account_deletion/pubspec.yaml` (`ente (word)`×1): Package name/description/repository metadata; not shown.
- `packages/accounts/lib/ente_accounts.dart` (`subscription`×1): only package imports (`ente_*` packages); internal.
- `packages/accounts/lib/models/bonus.dart` (`referral`×1):
  - L29 `static Set<String> signUpBonusTypes = {'SIGN_UP', 'REFERRAL'};`: Ente API path/JSON field (storage bonus / referral source); internal, the referral UI is deleted.
- `packages/accounts/lib/models/subscription.dart` (`subscription`×6): only identifiers (class/field/variable names such as `StreamSubscription`, `EnteFile`, `Subscription` model); internal.
- `packages/accounts/lib/models/user_details.dart` (`subscription`×11):
  - L119 `Subscription.fromMap(map['subscription']),`: Ente API path/JSON field (billing, user details); internal.
  - L134 `'subscription': subscription.toMap(),`: Ente API path/JSON field (billing, user details); internal.
  - remaining hits are package imports (`ente_*` packages), identifiers (class/field/variable names such as `StreamSubscription`, `EnteFile`, `Subscription` model); internal.
- `packages/accounts/lib/pages/email_entry_page.dart` (`ente.com`×2, `ente (word)`×2, `referral`×12): ente_accounts page: compiled in through the lock screen's barrel import, but Photos never opens it (it uses its own `lib/ui/account/` pages).
- `packages/accounts/lib/pages/login_page.dart` (`ente.com`×2, `ente (word)`×2): ente_accounts page: compiled in through the lock screen's barrel import, but Photos never opens it (it uses its own `lib/ui/account/` pages).
- `packages/accounts/lib/pages/login_pwd_verification_page.dart` (`ente.com`×1, `ente (word)`×1): ente_accounts page: compiled in through the lock screen's barrel import, but Photos never opens it (it uses its own `lib/ui/account/` pages).
- `packages/accounts/lib/pages/password_entry_page.dart` (`ente.com`×1, `ente (word)`×1): ente_accounts page: compiled in through the lock screen's barrel import, but Photos never opens it (it uses its own `lib/ui/account/` pages).
- `packages/accounts/lib/pages/password_reentry_page.dart` (`ente.com`×1, `ente (word)`×1): ente_accounts page: compiled in through the lock screen's barrel import, but Photos never opens it (it uses its own `lib/ui/account/` pages).
- `packages/accounts/lib/pages/recovery_key_page.dart` (`ente (word)`×1): ente_accounts page: compiled in through the lock screen's barrel import, but Photos never opens it (it uses its own `lib/ui/account/` pages).
- `packages/accounts/lib/pages/recovery_page.dart` (`ente.com`×1, `ente (word)`×1): ente_accounts page: compiled in through the lock screen's barrel import, but Photos never opens it (it uses its own `lib/ui/account/` pages).
- `packages/accounts/lib/pages/two_factor_recovery_page.dart` (`ente.com`×1, `ente (word)`×1): ente_accounts page: compiled in through the lock screen's barrel import, but Photos never opens it (it uses its own `lib/ui/account/` pages).
- `packages/accounts/lib/services/user_service.dart` (`referral`×4):
  - L40 `static const kReferralSource = "referral_source";`: Ente API path/JSON field (storage bonus / referral source); internal, the referral UI is deleted.
  - L379 `? '${_config.appIdentity.referralSourcePrefix}:${_getRefSource()}'`: Ente API path/JSON field (storage bonus / referral source); internal, the referral UI is deleted.
  - remaining hits are identifiers (class/field/variable names such as `StreamSubscription`, `EnteFile`, `Subscription` model); internal.
- `packages/accounts/pubspec.yaml` (`ente (word)`×1): Package name/description/repository metadata; not shown.
- `packages/backup_exclusion/ios/backup_exclusion.podspec` (`ente.com`×1, `ente (word)`×2): Native plugin podspec metadata (author e-mail/homepage); not shown.
- `packages/backup_exclusion/ios/backup_exclusion/Sources/backup_exclusion/BackupExclusionPlugin.swift` (`ente (word)`×1): Native plugin source/build ids (`io.ente.*` packages, class names, tests); not shown.
- `packages/backup_exclusion/lib/backup_exclusion.dart` (`ente (word)`×1):
  - L8 `const _channel = MethodChannel('io.ente.backup_exclusion');`: Platform identifier (namespace, bundle id, channel, task or app-group id); internal.
- `packages/backup_exclusion/pubspec.yaml` (`ente (word)`×1): Package name/description/repository metadata; not shown.
- `packages/backup_exclusion/test/backup_exclusion_test.dart` (`ente (word)`×1): Package test/example code; not shipped in Photos.
- `packages/base/pubspec.yaml` (`ente (word)`×1): Package name/description/repository metadata; not shown.
- `packages/configuration/lib/app_identity.dart` (`referral`×2): only identifiers (class/field/variable names such as `StreamSubscription`, `EnteFile`, `Subscription` model); internal.
- `packages/configuration/lib/constants.dart` (`ente.io`×1, `ente.com`×1, `ente (word)`×2):
  - L2 `const kLegacyProductionEndpoint = 'https://api.ente.io';`: Upstream production endpoint constants; the server comes from dart-define `PHOTOVAULT_SERVER` (or `endpoint`) / the connect screen via `setEndpoint()`; never a default.
  - L1 `const kDefaultProductionEndpoint = 'https://api.ente.com';`: Upstream production endpoint constants; the server comes from dart-define `PHOTOVAULT_SERVER` (or `endpoint`) / the connect screen via `setEndpoint()`; never a default.
- `packages/configuration/pubspec.yaml` (`ente (word)`×1): Package name/description/repository metadata; not shown.
- `packages/contacts/lib/src/db/contacts_database.dart` (`ente (word)`×1):
  - L14 `static const _databasePrefix = 'ente.contacts.';`: On-device database/index file name; internal.
- `packages/contacts/pubspec.yaml` (`ente (word)`×1): Package name/description/repository metadata; not shown.
- `packages/contacts/test/contacts_database_test.dart` (`ente (word)`×2): Package test/example code; not shipped in Photos.
- `packages/ente_components/assets/ente_wordmark.svg` (`ente (word)`×1): PHOTOVAULT header comment explaining the glyph replaced the mascot; artwork has no mascot.
- `packages/ente_components/example/android/app/build.gradle.kts` (`ente (word)`×2): Component catalog example app; not shipped.
- `packages/ente_components/example/android/app/src/main/kotlin/io/ente/ente_components_catalog/MainActivity.kt` (`ente (word)`×1): Component catalog example app; not shipped.
- `packages/ente_components/example/ios/Runner.xcodeproj/project.pbxproj` (`ente (word)`×6): Bundle ids `io.ente.frame*`, app-group ids, extension copyright, and stale Pods embed entries for Firebase/Sentry (regenerated by `pod install`). Not shown in the Android build; known iOS gap.
- `packages/ente_components/example/ios/Runner/Info.plist` (`ente (word)`×1): Component catalog example app; not shipped.
- `packages/ente_components/example/lib/main.dart` (`ente.io`×1, `ente (word)`×1, `referral`×1): Component catalog example app; not shipped.
- `packages/ente_components/lib/components/app_bar_component.dart` (`ente (word)`×2): only comments; internal.
- `packages/ente_components/lib/components/avatar_component.dart` (`ente (word)`×1): only comments; internal.
- `packages/ente_components/lib/components/banner_component.dart` (`ente (word)`×2): only comments; internal.
- `packages/ente_components/lib/components/bottom_sheet/bottom_sheet_component.dart` (`ente (word)`×3): only comments; internal.
- `packages/ente_components/lib/components/buttons/button_component.dart` (`ente (word)`×1): only comments; internal.
- `packages/ente_components/lib/components/buttons/fab_component.dart` (`ente (word)`×1): only comments; internal.
- `packages/ente_components/lib/components/buttons/icon_button_component.dart` (`ente (word)`×1): only comments; internal.
- `packages/ente_components/lib/components/divider_component.dart` (`ente (word)`×1): only comments; internal.
- `packages/ente_components/lib/components/filter_chip_component.dart` (`ente (word)`×1): only comments; internal.
- `packages/ente_components/lib/components/menu_component.dart` (`ente (word)`×1): only comments; internal.
- `packages/ente_components/lib/components/more_from_ente_section.dart` (`ente (word)`×4):
  - L25 `key: const ValueKey('more-from-ente'),`: `MoreFromEnteSection` in ente_components; Photos no longer uses it.
  - L61 `key: const ValueKey('more-from-ente-brand'),`: `MoreFromEnteSection` in ente_components; Photos no longer uses it.
  - L105 `label: 'Ente $label',`: `MoreFromEnteSection` in ente_components; Photos no longer uses it.
  - L115 `key: ValueKey('more-from-ente-${app.name}'),`: `MoreFromEnteSection` in ente_components; Photos no longer uses it.
- `packages/ente_components/lib/components/pin_input_component.dart` (`ente (word)`×2): only comments; internal.
- `packages/ente_components/lib/components/popup_menu_component.dart` (`ente (word)`×2):
  - L154 `key: ValueKey('ente-popup-menu-item-$index'),`: Internal temp-file / key prefix; not shown.
  - remaining hits are comments; internal.
- `packages/ente_components/lib/components/selection_controls/checkbox_component.dart` (`ente (word)`×1): only comments; internal.
- `packages/ente_components/lib/components/selection_controls/radio_component.dart` (`ente (word)`×1): only comments; internal.
- `packages/ente_components/lib/components/selection_controls/toggle_switch_component.dart` (`ente (word)`×1): only comments; internal.
- `packages/ente_components/lib/components/selection_summary_chip_component.dart` (`ente (word)`×2): only comments; internal.
- `packages/ente_components/lib/components/tag_chip_component.dart` (`ente (word)`×1): only comments; internal.
- `packages/ente_components/lib/components/text_input_component.dart` (`ente (word)`×2): only comments; internal.
- `packages/ente_components/lib/components/toast_component.dart` (`ente (word)`×1): only comments; internal.
- `packages/ente_components/lib/components/tooltip_component.dart` (`ente (word)`×1): only comments; internal.
- `packages/ente_components/lib/theme/colors.dart` (`ente (word)`×2): only comments; internal.
- `packages/ente_components/lib/theme/icon_sizes.dart` (`ente (word)`×1): only comments; internal.
- `packages/ente_components/lib/theme/motion.dart` (`ente (word)`×1): only comments; internal.
- `packages/ente_components/lib/theme/photovault/encore_color_tokens.dart` (`ente (word)`×5): only comments; internal.
- `packages/ente_components/lib/theme/photovault/encore_tokens.dart` (`ente (word)`×3):
  - L28 `static const String credit = 'Based on Ente';`: The required plain-text About credit ("Based on Ente").
  - remaining hits are comments; internal.
- `packages/ente_components/lib/theme/radii.dart` (`ente (word)`×1): only comments; internal.
- `packages/ente_components/lib/theme/shadows.dart` (`ente (word)`×1): only comments; internal.
- `packages/ente_components/lib/theme/spacing.dart` (`ente (word)`×1): only comments; internal.
- `packages/ente_components/lib/theme/text_styles.dart` (`ente (word)`×1): only comments; internal.
- `packages/ente_components/lib/theme/theme.dart` (`ente (word)`×1): only comments; internal.
- `packages/ente_components/test/components/popup_menu_component_test.dart` (`ente (word)`×2): Package test/example code; not shipped in Photos.
- `packages/ente_crypto_api/pubspec.yaml` (`ente (word)`×1): Package name/description/repository metadata; not shown.
- `packages/ente_crypto_dart_adapter/pubspec.yaml` (`ente (word)`×1): Package name/description/repository metadata; not shown.
- `packages/ente_icons/README.md` (`ente (word)`×1): Package documentation; not shipped.
- `packages/ente_icons/pubspec.yaml` (`ente (word)`×1): Package name/description/repository metadata; not shown.
- `packages/ente_pdf_renderer/android/build.gradle` (`ente (word)`×2): Native plugin source/build ids (`io.ente.*` packages, class names, tests); not shown.
- `packages/ente_pdf_renderer/android/src/main/kotlin/io/ente/pdf_renderer/EntePdfRendererPlugin.kt` (`ente (word)`×2): Native plugin source/build ids (`io.ente.*` packages, class names, tests); not shown.
- `packages/ente_pdf_renderer/ios/Classes/EntePdfRendererPlugin.swift` (`ente (word)`×2): Native plugin source/build ids (`io.ente.*` packages, class names, tests); not shown.
- `packages/ente_pdf_renderer/ios/ente_pdf_renderer.podspec` (`ente.io`×1, `ente (word)`×3): Native plugin podspec metadata (author e-mail/homepage); not shown.
- `packages/ente_pdf_renderer/lib/ente_pdf_renderer.dart` (`ente (word)`×1): Not compiled into Photos (no import path from `main.dart`; used by Ente Auth/Locker or unused).
- `packages/ente_pdf_renderer/pubspec.yaml` (`ente (word)`×1): Package name/description/repository metadata; not shown.
- `packages/ente_pdf_renderer/test/ente_pdf_renderer_test.dart` (`ente (word)`×1): Package test/example code; not shipped in Photos.
- `packages/ente_pure_utils/test/parse_util_test.dart` (`ente (word)`×5): Package test/example code; not shipped in Photos.
- `packages/ente_qr/android/build.gradle` (`ente (word)`×2): Native plugin source/build ids (`io.ente.*` packages, class names, tests); not shown.
- `packages/ente_qr/android/src/main/kotlin/io/ente/ente_qr/EnteQrPlugin.kt` (`ente (word)`×1): Native plugin source/build ids (`io.ente.*` packages, class names, tests); not shown.
- `packages/ente_qr/ios/Classes/EnteQrPlugin.swift` (`ente (word)`×1): Native plugin source/build ids (`io.ente.*` packages, class names, tests); not shown.
- `packages/ente_qr/ios/ente_qr.podspec` (`ente.com`×2, `ente (word)`×4): Native plugin podspec metadata (author e-mail/homepage); not shown.
- `packages/ente_qr/macos/Classes/EnteQrPlugin.swift` (`ente (word)`×1): Native plugin source/build ids (`io.ente.*` packages, class names, tests); not shown.
- `packages/ente_qr/macos/ente_qr.podspec` (`ente.com`×2, `ente (word)`×4): Native plugin podspec metadata (author e-mail/homepage); not shown.
- `packages/ente_qr/pubspec.yaml` (`ente (word)`×2): Package name/description/repository metadata; not shown.
- `packages/ente_qr_scanner/android/build.gradle` (`ente (word)`×2): Native plugin source/build ids (`io.ente.*` packages, class names, tests); not shown.
- `packages/ente_qr_scanner/android/src/main/kotlin/io/ente/qr_scanner/EnteQrScannerOverlay.kt` (`ente (word)`×1): Native plugin source/build ids (`io.ente.*` packages, class names, tests); not shown.
- `packages/ente_qr_scanner/android/src/main/kotlin/io/ente/qr_scanner/EnteQrScannerPlugin.kt` (`ente (word)`×1): Native plugin source/build ids (`io.ente.*` packages, class names, tests); not shown.
- `packages/ente_qr_scanner/android/src/main/kotlin/io/ente/qr_scanner/EnteQrScannerView.kt` (`ente (word)`×2): Native plugin source/build ids (`io.ente.*` packages, class names, tests); not shown.
- `packages/ente_qr_scanner/android/src/main/kotlin/io/ente/qr_scanner/EnteQrScannerViewFactory.kt` (`ente (word)`×2): Native plugin source/build ids (`io.ente.*` packages, class names, tests); not shown.
- `packages/ente_qr_scanner/android/src/main/kotlin/io/ente/qr_scanner/QrFrameAnalyzer.kt` (`ente (word)`×1): Native plugin source/build ids (`io.ente.*` packages, class names, tests); not shown.
- `packages/ente_qr_scanner/ios/Classes/EnteQrScannerPlugin.swift` (`ente (word)`×1): Native plugin source/build ids (`io.ente.*` packages, class names, tests); not shown.
- `packages/ente_qr_scanner/ios/Classes/EnteQrScannerView.swift` (`ente (word)`×3): Native plugin source/build ids (`io.ente.*` packages, class names, tests); not shown.
- `packages/ente_qr_scanner/ios/ente_qr_scanner.podspec` (`ente.com`×2, `ente (word)`×4): Native plugin podspec metadata (author e-mail/homepage); not shown.
- `packages/ente_qr_scanner/lib/src/ente_qr_scanner_controller.dart` (`ente (word)`×2): Not compiled into Photos (no import path from `main.dart`; used by Ente Auth/Locker or unused).
- `packages/ente_qr_scanner/lib/src/ente_qr_scanner_view.dart` (`ente (word)`×1): Not compiled into Photos (no import path from `main.dart`; used by Ente Auth/Locker or unused).
- `packages/ente_qr_scanner/pubspec.yaml` (`ente (word)`×2): Package name/description/repository metadata; not shown.
- `packages/ente_screen_brightness/ios/Classes/EnteScreenBrightnessPlugin.swift` (`ente (word)`×1): Native plugin source/build ids (`io.ente.*` packages, class names, tests); not shown.
- `packages/ente_screen_brightness/ios/ente_screen_brightness.podspec` (`ente.io`×1, `ente (word)`×3): Native plugin podspec metadata (author e-mail/homepage); not shown.
- `packages/ente_screen_brightness/lib/ente_screen_brightness.dart` (`ente (word)`×1):
  - L9 `"io.ente.screen_brightness",`: Platform identifier (namespace, bundle id, channel, task or app-group id); internal.
- `packages/ente_screen_brightness/pubspec.yaml` (`ente (word)`×1): Package name/description/repository metadata; not shown.
- `packages/feature_flag/lib/src/remote_flags.dart` (`ente (word)`×1): only comments; internal.
- `packages/frb/pubspec.yaml` (`ente (word)`×1): Package name/description/repository metadata; not shown.
- `packages/install_source/android/build.gradle` (`ente (word)`×2): Native plugin source/build ids (`io.ente.*` packages, class names, tests); not shown.
- `packages/install_source/android/src/main/kotlin/io/ente/install_source/EnteInstallSourcePlugin.kt` (`ente (word)`×2): Native plugin source/build ids (`io.ente.*` packages, class names, tests); not shown.
- `packages/install_source/android/src/main/kotlin/io/ente/install_source/InstallSourceEventStore.kt` (`ente (word)`×1): Native plugin source/build ids (`io.ente.*` packages, class names, tests); not shown.
- `packages/install_source/android/src/noReferrer/kotlin/io/ente/install_source/InstallSourceProvider.kt` (`ente (word)`×1): Native plugin source/build ids (`io.ente.*` packages, class names, tests); not shown.
- `packages/install_source/android/src/playstore/kotlin/io/ente/install_source/InstallSourceProvider.kt` (`ente (word)`×1): Native plugin source/build ids (`io.ente.*` packages, class names, tests); not shown.
- `packages/install_source/lib/src/install_source_service.dart` (`ente (word)`×1):
  - L18 `static const _channelName = 'io.ente.install_source/install_source';`: Platform identifier (namespace, bundle id, channel, task or app-group id); internal.
- `packages/install_source/pubspec.yaml` (`ente (word)`×2): Package name/description/repository metadata; not shown.
- `packages/legacy/assets/legacy_kit_sheet_ente_logo_black.svg` (`ente (word)`×1): PHOTOVAULT header comment explaining the glyph replaced the mascot; artwork has no mascot.
- `packages/legacy/lib/services/legacy_kit_pdf_service.dart` (`ente.com`×2, `ente (word)`×8): ente_legacy package (Locker legacy kit); not compiled into Photos.
- `packages/legacy/lib/services/legacy_kit_share_file_service.dart` (`ente (word)`×1): ente_legacy package (Locker legacy kit); not compiled into Photos.
- `packages/legacy/pubspec.yaml` (`ente (word)`×1): Package name/description/repository metadata; not shown.
- `packages/legacy/test/legacy_kit_pdf_service_test.dart` (`ente.com`×5, `ente (word)`×5): ente_legacy package (Locker legacy kit); not compiled into Photos.
- `packages/local_auth_linux/lib/local_auth_linux.dart` (`ente.io`×1, `ente (word)`×4): Linux-only local auth plugin (Ente Auth); unreachable on Android/iOS.
- `packages/local_auth_linux/linux/local_auth_linux_plugin.cc` (`ente.io`×1, `ente (word)`×6): Native plugin source/build ids (`io.ente.*` packages, class names, tests); not shown.
- `packages/local_auth_linux/pubspec.yaml` (`ente (word)`×1): Package name/description/repository metadata; not shown.
- `packages/local_auth_linux/test/local_auth_linux_test.dart` (`ente (word)`×9): Linux-only local auth plugin (Ente Auth); unreachable on Android/iOS.
- `packages/lock_screen/lib/auth_util.dart` (`ente (word)`×2):
  - L57 `"Linux system authentication needs one-time setup. Install the Ente Auth Polkit policy, or`: Ente Auth specific (package-name check, Linux polkit id/path); not used by Photos on Android/iOS.
  - remaining hits are comments; internal.
- `packages/lock_screen/lib/ui/local_authentication_unavailable_dialog.dart` (`ente.com`×1, `ente (word)`×2): Linux-only lock-screen help (Ente Auth polkit setup); unreachable on Android/iOS.
- `packages/lock_screen/pubspec.yaml` (`ente (word)`×1): Package name/description/repository metadata; not shown.
- `packages/logging/lib/logging.dart` (`sentry`×1): only comments; internal.
- `packages/logging/lib/super_logging.dart` (`sentry`×9): only comments, identifiers (class/field/variable names such as `StreamSubscription`, `EnteFile`, `Subscription` model); internal.
- `packages/logging/pubspec.yaml` (`ente (word)`×1, `sentry`×1): Package name/description/repository metadata; not shown.
- `packages/mail/android/build.gradle` (`ente (word)`×2): Native plugin source/build ids (`io.ente.*` packages, class names, tests); not shown.
- `packages/mail/android/src/main/kotlin/io/ente/mail/MailFileProvider.kt` (`ente (word)`×1): Native plugin source/build ids (`io.ente.*` packages, class names, tests); not shown.
- `packages/mail/android/src/main/kotlin/io/ente/mail/MailPlugin.kt` (`ente (word)`×7): Native plugin source/build ids (`io.ente.*` packages, class names, tests); not shown.
- `packages/mail/android/src/main/kotlin/io/ente/mail/core/AttachmentStore.kt` (`ente (word)`×1): Native plugin source/build ids (`io.ente.*` packages, class names, tests); not shown.
- `packages/mail/android/src/main/kotlin/io/ente/mail/core/MailComposer.kt` (`ente (word)`×1): Native plugin source/build ids (`io.ente.*` packages, class names, tests); not shown.
- `packages/mail/android/src/main/kotlin/io/ente/mail/core/MailModels.kt` (`ente (word)`×1): Native plugin source/build ids (`io.ente.*` packages, class names, tests); not shown.
- `packages/mail/android/src/test/kotlin/io/ente/mail/core/AttachmentStoreTest.kt` (`ente (word)`×3): Native plugin source/build ids (`io.ente.*` packages, class names, tests); not shown.
- `packages/mail/android/src/test/kotlin/io/ente/mail/core/MailModelsTest.kt` (`ente (word)`×1): Native plugin source/build ids (`io.ente.*` packages, class names, tests); not shown.
- `packages/mail/ios/Tests/MailModelsTests.swift` (`ente.io`×2, `ente (word)`×2): Native plugin source/build ids (`io.ente.*` packages, class names, tests); not shown.
- `packages/mail/ios/ente_mail.podspec` (`ente.io`×1, `ente (word)`×3): Native plugin podspec metadata (author e-mail/homepage); not shown.
- `packages/mail/ios/ente_mail/Package.swift` (`ente (word)`×1): Native plugin source/build ids (`io.ente.*` packages, class names, tests); not shown.
- `packages/mail/ios/ente_mail/Sources/ente_mail/MailPlugin.swift` (`ente (word)`×1): Native plugin source/build ids (`io.ente.*` packages, class names, tests); not shown.
- `packages/mail/lib/ente_mail.dart` (`ente (word)`×1):
  - L72 `MethodChannel channel = const MethodChannel('io.ente.mail/composer'),`: Platform identifier (namespace, bundle id, channel, task or app-group id); internal.
- `packages/mail/pubspec.yaml` (`ente (word)`×2): Package name/description/repository metadata; not shown.
- `packages/mail/test/ente_mail_test.dart` (`ente.io`×4, `ente (word)`×4): Package test/example code; not shipped in Photos.
- `packages/native_video_editor/android/build.gradle` (`ente (word)`×2): Native plugin source/build ids (`io.ente.*` packages, class names, tests); not shown.
- `packages/native_video_editor/android/src/main/kotlin/io/ente/native_video_editor/NativeVideoEditorPlugin.kt` (`ente (word)`×11): Native plugin source/build ids (`io.ente.*` packages, class names, tests); not shown.
- `packages/native_video_editor/android/src/main/kotlin/io/ente/native_video_editor/core/VideoExportEngine.kt` (`ente (word)`×1): Native plugin source/build ids (`io.ente.*` packages, class names, tests); not shown.
- `packages/native_video_editor/android/src/main/kotlin/io/ente/native_video_editor/core/VideoFrameExtractor.kt` (`ente (word)`×1): Native plugin source/build ids (`io.ente.*` packages, class names, tests); not shown.
- `packages/native_video_editor/android/src/main/kotlin/io/ente/native_video_editor/core/VideoMetadataReader.kt` (`ente (word)`×1): Native plugin source/build ids (`io.ente.*` packages, class names, tests); not shown.
- `packages/native_video_editor/android/src/main/kotlin/io/ente/native_video_editor/core/VideoModels.kt` (`ente (word)`×1): Native plugin source/build ids (`io.ente.*` packages, class names, tests); not shown.
- `packages/native_video_editor/android/src/test/kotlin/io/ente/native_video_editor/VideoClipRangeUsTest.kt` (`ente (word)`×1): Native plugin source/build ids (`io.ente.*` packages, class names, tests); not shown.
- `packages/native_video_editor/android/src/test/kotlin/io/ente/native_video_editor/VideoModelsTest.kt` (`ente (word)`×1): Native plugin source/build ids (`io.ente.*` packages, class names, tests); not shown.
- `packages/native_video_editor/ios/native_video_editor.podspec` (`ente.com`×1, `ente (word)`×2): Native plugin podspec metadata (author e-mail/homepage); not shown.
- `packages/native_video_editor/lib/native_video_editor.dart` (`subscription`×2): only identifiers (class/field/variable names such as `StreamSubscription`, `EnteFile`, `Subscription` model); internal.
- `packages/native_video_editor/pubspec.yaml` (`ente (word)`×2): Package name/description/repository metadata; not shown.
- `packages/network/lib/network.dart` (`ente (word)`×5):
  - L31 `if (packageName == 'ente_auth' || packageName == 'Ente Auth') {`: Ente Auth specific (package-name check, Linux polkit id/path); not used by Photos on Android/iOS.
  - L32 `packageName = 'io.ente.auth';`: Platform identifier (namespace, bundle id, channel, task or app-group id); internal.
  - L37 `if (!packageName.startsWith('io.ente.')) {`: Platform identifier (namespace, bundle id, channel, task or app-group id); internal.
  - L40 `'Expected package name to start with "io.ente." but got "$packageName". '`: Platform identifier (namespace, bundle id, channel, task or app-group id); internal.
  - remaining hits are comments; internal.
- `packages/qr/pubspec.yaml` (`ente (word)`×2): Package name/description/repository metadata; not shown.
- `packages/scoped_dir_access/ios/Classes/DirUtilsPlugin.swift` (`ente (word)`×1): Native plugin source/build ids (`io.ente.*` packages, class names, tests); not shown.
- `packages/scoped_dir_access/ios/scoped_dir_access.podspec` (`ente.com`×1, `ente (word)`×2): Native plugin podspec metadata (author e-mail/homepage); not shown.
- `packages/scoped_dir_access/lib/scoped_dir_access.dart` (`ente (word)`×1): Not compiled into Photos (no import path from `main.dart`; used by Ente Auth/Locker or unused).
- `packages/scoped_dir_access/macos/Classes/DirUtilsPlugin.swift` (`ente (word)`×1): Native plugin source/build ids (`io.ente.*` packages, class names, tests); not shown.
- `packages/scoped_dir_access/macos/scoped_dir_access.podspec` (`ente.com`×1, `ente (word)`×2): Native plugin podspec metadata (author e-mail/homepage); not shown.
- `packages/screen_cover/android/build.gradle` (`ente (word)`×2): Native plugin source/build ids (`io.ente.*` packages, class names, tests); not shown.
- `packages/screen_cover/android/src/main/kotlin/io/ente/ente_screen_cover/EnteScreenCoverPlugin.kt` (`ente (word)`×1): Native plugin source/build ids (`io.ente.*` packages, class names, tests); not shown.
- `packages/screen_cover/ios/ente_screen_cover.podspec` (`ente.com`×2, `ente (word)`×3): Native plugin podspec metadata (author e-mail/homepage); not shown.
- `packages/screen_cover/pubspec.yaml` (`ente (word)`×1): Package name/description/repository metadata; not shown.
- `packages/sharing/pubspec.yaml` (`ente (word)`×1): Package name/description/repository metadata; not shown.
- `packages/sharing/test/user_avatar_widget_test.dart` (`ente (word)`×1, `referral`×1): Package test/example code; not shipped in Photos.
- `packages/strings/README.md` (`ente (word)`×3): Package documentation; not shipped.
- `packages/strings/pubspec.yaml` (`ente (word)`×1): Package name/description/repository metadata; not shown.
- `packages/ui/lib/components/settings/about_settings_section.dart` (`ente.com`×3, `ente (word)`×4): Not compiled into Photos (no import path from `main.dart`; used by Ente Auth/Locker or unused).
- `packages/ui/lib/components/settings/app_engagement_section.dart` (`ente.com`×1, `ente (word)`×1, `merch`×3): Not compiled into Photos (no import path from `main.dart`; used by Ente Auth/Locker or unused).
- `packages/ui/lib/components/settings/more_from_ente_footer.dart` (`ente.com`×1, `ente (word)`×1): Not compiled into Photos (no import path from `main.dart`; used by Ente Auth/Locker or unused).
- `packages/ui/lib/components/settings/social_icons_row.dart` (`ente.com`×1, `ente (word)`×3): Not compiled into Photos (no import path from `main.dart`; used by Ente Auth/Locker or unused).
- `packages/ui/lib/components/subscription/plan_card.dart` (`subscription`×4): Not compiled into Photos (no import path from `main.dart`; used by Ente Auth/Locker or unused).
- `packages/ui/lib/components/subscription/plan_price.dart` (`subscription`×2): Not compiled into Photos (no import path from `main.dart`; used by Ente Auth/Locker or unused).
- `packages/ui/lib/utils/dialog_util.dart` (`ente.com`×4, `ente (word)`×4): ente_ui dialogs Photos does not call (Photos uses its own `lib/utils/dialog_util.dart`; it imports only `showChoiceDialog` from here).
- `packages/ui/pubspec.yaml` (`ente (word)`×1): Package name/description/repository metadata; not shown.
- `packages/utils/lib/email_util.dart` (`ente (word)`×2): ente_utils log/e-mail helper; Photos does not call it (it has its own `lib/utils/email_util.dart`).
- `packages/utils/pubspec.yaml` (`ente (word)`×1): Package name/description/repository metadata; not shown.

## Strings (`packages/strings/lib/l10n/arb/strings_*.arb`)

Grouped per message key across all 57 locales. "used" means Photos or a package it compiles references the key; unused keys are kept from upstream (Ente Auth/Locker or removed Photos UI) and are never displayed by Photovault. Placeholder-description metadata (`@key` blocks) is translator-only.

- `@albumsOnEnteEmptyDescription` (7 locales; `ente (word)`×7): Translator metadata (description/placeholder example); never displayed.
- `@cancelOtherSubscription` (33 locales; `subscription`×66): Translator metadata (description/placeholder example); never displayed.
- `@castInstruction` (14 locales; `ente.com`×14, `ente (word)`×14): Translator metadata (description/placeholder example); never displayed.
- `@contactToManageSubscription` (17 locales; `subscription`×17): Translator metadata (description/placeholder example); never displayed.
- `@contactUs` (11 locales; `ente (word)`×11): Translator metadata (description/placeholder example); never displayed.
- `@dropSupportEmail` (41 locales; `ente.com`×41, `ente (word)`×41): Translator metadata (description/placeholder example); never displayed.
- `@filesInAlbumSafelyOnEnte` (9 locales; `ente (word)`×9): Translator metadata (description/placeholder example); never displayed.
- `@filesSafelyOnEnte` (9 locales; `ente (word)`×9): Translator metadata (description/placeholder example); never displayed.
- `@goToUrl` (9 locales; `ente.com`×9, `ente (word)`×9): Translator metadata (description/placeholder example); never displayed.
- `@hearUsWhereOptionalTitle` (37 locales; `ente (word)`×37): Translator metadata (description/placeholder example); never displayed.
- `@legacyKitSheetNeedHelp` (10 locales; `ente.com`×10, `ente (word)`×10): Translator metadata (description/placeholder example); never displayed.
- `@legacyKitSheetStepVisit` (10 locales; `ente.com`×10, `ente (word)`×10): Translator metadata (description/placeholder example); never displayed.
- `@loadMessage8` (30 locales; `ente.com`×30, `ente (word)`×30): Translator metadata (description/placeholder example); never displayed.
- `@meetEnteLocker` (7 locales; `ente (word)`×7): Translator metadata (description/placeholder example); never displayed.
- `@moreFrom` (11 locales; `ente (word)`×11): Translator metadata (description/placeholder example); never displayed.
- `@networkConnectionRefusedErr` (30 locales; `ente (word)`×30): Translator metadata (description/placeholder example); never displayed.
- `@networkHostLookUpErr` (30 locales; `ente (word)`×30): Translator metadata (description/placeholder example); never displayed.
- `@noEmailAppBody` (17 locales; `ente.com`×17, `ente (word)`×17): Translator metadata (description/placeholder example); never displayed.
- `@onEnte` (33 locales; `ente (word)`×33): Translator metadata (description/placeholder example); never displayed.
- `@oneAccountAcrossEnteApps` (10 locales; `ente (word)`×10): Translator metadata (description/placeholder example); never displayed.
- `@referralStep2` (9 locales; `referral`×9): Translator metadata (description/placeholder example); never displayed.
- `@referralStep3` (9 locales; `referral`×9): Translator metadata (description/placeholder example); never displayed.
- `@referralStorageForBoth` (22 locales; `referral`×22): Translator metadata (description/placeholder example); never displayed.
- `@selectedFilesBackedUpToEnte` (15 locales; `ente (word)`×15): Translator metadata (description/placeholder example); never displayed.
- `@sendToEmail` (10 locales; `ente.com`×10, `ente (word)`×10): Translator metadata (description/placeholder example); never displayed.
- `@shareTextReferralInvite` (9 locales; `referral`×18): Translator metadata (description/placeholder example); never displayed.
- `@singleFileInBothLocalAndRemote` (14 locales; `ente (word)`×14): Translator metadata (description/placeholder example); never displayed.
- `@singleFileInRemoteOnly` (13 locales; `ente (word)`×13): Translator metadata (description/placeholder example); never displayed.
- `@someSelectedFilesBackedUpToEnte` (14 locales; `ente (word)`×14): Translator metadata (description/placeholder example); never displayed.
- `@subscriptionContinueWithPlan` (1 locales; `subscription`×1): Translator metadata (description/placeholder example); never displayed.
- `@subscriptionLockerItems` (1 locales; `subscription`×1): Translator metadata (description/placeholder example); never displayed.
- `@thisIsPersonVerificationId` (35 locales; `ente.com`×35, `ente (word)`×35): Translator metadata (description/placeholder example); never displayed.
- `@uploadSubscriptionExpiredErrorBody` (11 locales; `subscription`×22): Translator metadata (description/placeholder example); never displayed.
- `@uploadSubscriptionExpiredErrorTitle` (12 locales; `subscription`×24): Translator metadata (description/placeholder example); never displayed.
- `@visitWebToManage` (32 locales; `ente.com`×32, `ente (word)`×32, `subscription`×32): Translator metadata (description/placeholder example); never displayed.
- `@weHaveSentCodeTo` (30 locales; `ente.com`×30, `ente (word)`×30): Translator metadata (description/placeholder example); never displayed.
- `aNewVersionOfEnteLockerIsAvailable` (17 locales; `ente (word)`×17): Unused key (Ente Auth/Locker or removed Photos UI); never displayed by Photovault.
- `appstoreSubscription` (31 locales; `subscription`×31): Unused key (Ente Auth/Locker or removed Photos UI); never displayed by Photovault.
- `askCancelReason` (2 locales; `subscription`×2): Unused key (Ente Auth/Locker or removed Photos UI); never displayed by Photovault.
- `cancelOtherSubscription` (33 locales; `subscription`×33): Unused key (Ente Auth/Locker or removed Photos UI); never displayed by Photovault.
- `cancelSubscription` (33 locales; `subscription`×33): Unused key (Ente Auth/Locker or removed Photos UI); never displayed by Photovault.
- `changeYourReferralCode` (26 locales; `referral`×26): Unused key (Ente Auth/Locker or removed Photos UI); never displayed by Photovault.
- `contactToManageSubscription` (17 locales; `ente.com`×17, `ente (word)`×17, `subscription`×17): Unused key (Ente Auth/Locker or removed Photos UI); never displayed by Photovault.
- `couldNotUpdateSubscription` (34 locales; `subscription`×34): Unused key (Ente Auth/Locker or removed Photos UI); never displayed by Photovault.
- `discover_pets` (2 locales; `mascot`×2): Matches Spanish/Portuguese "mascotas" (pets), not the mascot.
- `emailVerificationEnableWarning` (31 locales; `ente (word)`×31): Unused key (Ente Auth/Locker or removed Photos UI); never displayed by Photovault.
- `ente` (18 locales; `ente (word)`×18): Key name only; the value is "Photovault" in every locale (album search section title).
- `enteAuth` (13 locales; `ente (word)`×13): Unused key (Ente Auth/Locker or removed Photos UI); never displayed by Photovault.
- `enteLocker` (15 locales; `ente (word)`×15): Unused key (Ente Auth/Locker or removed Photos UI); never displayed by Photovault.
- `enterReferralCode` (34 locales; `referral`×34): Unused key (Ente Auth/Locker or removed Photos UI); never displayed by Photovault.
- `failedToFetchReferralDetails` (34 locales; `referral`×34): Unused key (Ente Auth/Locker or removed Photos UI); never displayed by Photovault.
- `failedToRefreshStripeSubscription` (29 locales; `subscription`×29): Unused key (Ente Auth/Locker or removed Photos UI); never displayed by Photovault.
- `free10GB` (1 locales; `ente (word)`×1): Unused key (Ente Auth/Locker or removed Photos UI); never displayed by Photovault.
- `freeStorageOffer` (1 locales; `ente (word)`×1): Unused key (Ente Auth/Locker or removed Photos UI); never displayed by Photovault.
- `importTypeEnteEncrypted` (1 locales; `ente (word)`×1): Unused key (Ente Auth/Locker or removed Photos UI); never displayed by Photovault.
- `invalidReferralCode` (28 locales; `referral`×28): Unused key (Ente Auth/Locker or removed Photos UI); never displayed by Photovault.
- `loadMessage1` (2 locales; `subscription`×2): Unused key (Ente Auth/Locker or removed Photos UI); never displayed by Photovault.
- `loginWithAuthAccount` (2 locales; `ente (word)`×2): Unused key (Ente Auth/Locker or removed Photos UI); never displayed by Photovault.
- `manageSubscription` (38 locales; `subscription`×38): Unused key (Ente Auth/Locker or removed Photos UI); never displayed by Photovault.
- `meetEnteLocker` (7 locales; `ente (word)`×7): Unused key (Ente Auth/Locker or removed Photos UI); never displayed by Photovault.
- `merchandise` (35 locales; `merch`×35): Used only by `AppEngagementSection` in ente_ui, which Photos no longer shows.
- `offlineKeyUnavailableMessage` (9 locales; `ente (word)`×9): Unused key (Ente Auth/Locker or removed Photos UI); never displayed by Photovault.
- `playstoreSubscription` (32 locales; `subscription`×32): Unused key (Ente Auth/Locker or removed Photos UI); never displayed by Photovault.
- `pleaseContactSupportAndWeWillBeHappyToHelp` (20 locales; `ente.com`×20, `ente (word)`×20): Unused key (Ente Auth/Locker or removed Photos UI); never displayed by Photovault.
- `rateUs` (2 locales; `rate us`×2): Used only by `AppEngagementSection` in ente_ui, which Photos no longer shows.
- `rateUsOnStore` (1 locales; `rate us`×1): Unused key (Ente Auth/Locker or removed Photos UI); never displayed by Photovault.
- `referralCodeHint` (26 locales; `referral`×26): Unused key (Ente Auth/Locker or removed Photos UI); never displayed by Photovault.
- `referralStats` (24 locales; `referral`×24): JSON field name match only (Photos reads it from the bonus API model); referral UI deleted.
- `referralStep1` (9 locales; `referral`×9): Unused key (Ente Auth/Locker or removed Photos UI); never displayed by Photovault.
- `referralStep2` (9 locales; `referral`×9): Unused key (Ente Auth/Locker or removed Photos UI); never displayed by Photovault.
- `referralStep3` (9 locales; `referral`×9): Unused key (Ente Auth/Locker or removed Photos UI); never displayed by Photovault.
- `referralStorageForBoth` (22 locales; `referral`×22): Unused key (Ente Auth/Locker or removed Photos UI); never displayed by Photovault.
- `referralStorageInfo` (22 locales; `referral`×22): Unused key (Ente Auth/Locker or removed Photos UI); never displayed by Photovault.
- `referrals` (33 locales; `referral`×33): Unused key (Ente Auth/Locker or removed Photos UI); never displayed by Photovault.
- `referralsAreCurrentlyPaused` (34 locales; `referral`×34): Unused key (Ente Auth/Locker or removed Photos UI); never displayed by Photovault.
- `renewSubscription` (31 locales; `subscription`×31): Unused key (Ente Auth/Locker or removed Photos UI); never displayed by Photovault.
- `renewsOn` (2 locales; `subscription`×2): Unused key (Ente Auth/Locker or removed Photos UI); never displayed by Photovault.
- `shareTextReferralInvite` (9 locales; `ente.com`×9, `ente (word)`×9, `referral`×9): Unused key (Ente Auth/Locker or removed Photos UI); never displayed by Photovault.
- `shareYourSubscription` (22 locales; `subscription`×22): Unused key (Ente Auth/Locker or removed Photos UI); never displayed by Photovault.
- `subWillBeCancelledOn` (2 locales; `subscription`×2): Unused key (Ente Auth/Locker or removed Photos UI); never displayed by Photovault.
- `subscription` (32 locales; `subscription`×32): Not displayed by Photos (its subscription UI is deleted); counted as used only because `.subscription` is also a model field.
- `subscriptionContinueWithPlan` (1 locales; `subscription`×1): Unused key (Ente Auth/Locker or removed Photos UI); never displayed by Photovault.
- `subscriptionCurrentPlan` (1 locales; `subscription`×1): Unused key (Ente Auth/Locker or removed Photos UI); never displayed by Photovault.
- `subscriptionFamilyDescription` (1 locales; `subscription`×1): Unused key (Ente Auth/Locker or removed Photos UI); never displayed by Photovault.
- `subscriptionFaqDescription` (1 locales; `subscription`×1): Unused key (Ente Auth/Locker or removed Photos UI); never displayed by Photovault.
- `subscriptionLearnMore` (1 locales; `subscription`×1): Unused key (Ente Auth/Locker or removed Photos UI); never displayed by Photovault.
- `subscriptionLockerDescription` (1 locales; `subscription`×1): Unused key (Ente Auth/Locker or removed Photos UI); never displayed by Photovault.
- `subscriptionLockerItems` (1 locales; `subscription`×1): Unused key (Ente Auth/Locker or removed Photos UI); never displayed by Photovault.
- `subscriptionLockerTitle` (1 locales; `subscription`×1): Unused key (Ente Auth/Locker or removed Photos UI); never displayed by Photovault.
- `subscriptionRequiredForSharing` (24 locales; `subscription`×24): Unused key (Ente Auth/Locker or removed Photos UI); never displayed by Photovault.
- `supportEnte` (2 locales; `ente.io`×1, `ente (word)`×2): Unused key (Ente Auth/Locker or removed Photos UI); never displayed by Photovault.
- `unavailableReferralCode` (9 locales; `referral`×9): Unused key (Ente Auth/Locker or removed Photos UI); never displayed by Photovault.
- `uploadSubscriptionExpiredErrorBody` (11 locales; `subscription`×11): Unused key (Ente Auth/Locker or removed Photos UI); never displayed by Photovault.
- `uploadSubscriptionExpiredErrorTitle` (12 locales; `subscription`×12): Unused key (Ente Auth/Locker or removed Photos UI); never displayed by Photovault.
- `visitWebToManage` (2 locales; `subscription`×2): Unused key (Ente Auth/Locker or removed Photos UI); never displayed by Photovault.
- `yourReferralCode` (10 locales; `referral`×10): Unused key (Ente Auth/Locker or removed Photos UI); never displayed by Photovault.
- `yourSubscriptionHasExpired` (32 locales; `subscription`×32): Unused key (Ente Auth/Locker or removed Photos UI); never displayed by Photovault.
- `yourSubscriptionWasUpdatedSuccessfully` (33 locales; `subscription`×33): Unused key (Ente Auth/Locker or removed Photos UI); never displayed by Photovault.

## Build output (APK) grep

**The arm64 release APK has not been built yet**, so the planned grep of
`classes*.dex`, `resources.arsc`, `assets/` and `lib/arm64-v8a/libapp.so` has
not run. The build environment cannot reach `dl.google.com` (Android SDK, NDK
and Google's Maven repository), so `flutter build apk` stops at "No Android SDK
found". Checked again on 2026-11-04 after the review fixes: `dl.google.com`
is still refused by the proxy (`maven.google.com` answers, but the SDK
platform, build-tools and NDK come only from `dl.google.com`), so there is
still no APK. When an APK exists, `.github/workflows/photovault-mobile.yml` (step
"Check APK") fails the build if the dex contains `com/google/firebase`,
`com/google/android/gms`, `io/sentry` or `com/android/billingclient` classes,
and checks the applicationId and signature.

Done instead, as a partial check: `flutter build bundle --target-platform=linux-x64
--dart-define=cronetHttpNoPlay=true --dart-define=PHOTOVAULT_SERVER=` gives the
same `flutter_assets/` as the APK (images, fonts, shaders, JSON) and a debug
`kernel_blob.bin` holding all Dart code and strings Photos imports. The
debug kernel is a superset of the release `libapp.so`: it is not tree-shaken and
it embeds Dart source text, comments included. Counts from `strings -n 4` over
the kernel (case-insensitive):

| pattern | hits | what they are |
|---|---|---|
| `play.google`, `apps.apple` | 0 | none |
| `com.google.android.gms`, `billingclient` | 0 | none |
| `firebase` | 6 | comments, plus the log tags `firebaseBgSync*` and `via: 'firebasePush'` in `lib/main.dart` (background-sync entry points that Ente's F-Droid stub also uses; no Firebase code) |
| `sentry` | 64 | `// PHOTOVAULT: telemetry removed` comments, `logging`'s `sentryIsEnabled` field (always false), and unrelated identifiers (`entriesentry`, `SoundSentry` from `win32`) |
| `ente.io` | 10 | `kLegacyProductionEndpoint` / `_isPublicAlbumHost` constants (recognize old Ente links, not used as defaults), `ente.io/local_auth_linux` (a plugin's channel name), pt_BR `supportEnte` (unused key, see above) |
| `ente.com` | 267 | almost all from the ARB strings (all 57 locales are compiled in): the referral invite text, `support@ente.com` in subscription/support messages, both in unused keys listed above; `kDefaultProductionEndpoint` (only compared against, never a default); `sendLogs(context, "support@ente.com")` in shared `ente_ui` code Photos does not call |
| `referral`, `subscription`, `merch`, `rate us`, `duck`, `mascot` | 1136, 3753, 89, 57, 171, 14 | identifiers, ARB keys and messages of removed screens, and asset file names; see the source sections above. `rate us` hits are the generated getters for the removed Rate us entry's strings, a `PHOTOVAULT` comment, and unrelated text such as "migrate using" |

Assets in `flutter_assets/`: no JSON, SVG, XML, HTML or JS asset contains
`ente.io`, `ente.com`, `firebase`, `sentry`, `play.google`, `apps.apple`,
`referral` or `subscription`. File names still contain `ducky` (for example
`assets/ducky_share.png` and `ducky_cleaning_static.svg`). They keep Ente's names so
upstream code paths stay the same, but the images are the re-rendered
Photovault glyph art (see "Mascot" in `PHOTOVAULT.md`). No `.riv` (Rive) file
is bundled.
