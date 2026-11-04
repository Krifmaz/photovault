# Decisions

One line per non-obvious call, newest last.

- 2026-11-04: Backup progress notification (Photos app): the app is Flutter, so Krifmaz/kit's kit-compose (Jetpack Compose) widgets cannot render in it and linking the library would put kit's AGP/Kotlin/Compose pins into Flutter's Gradle build; the banner and copyable error details are Flutter widgets that follow kit's rules (plain words, errors copyable), and no kit piece was added because no app would use it yet. Photovault Native (Compose) is the place to use kit's notices/toasts if it gets the same feature.
- 2026-11-04: Backup notification runs the app as a `dataSync` foreground service only while uploading; paused, failed and finished states are plain notifications, so no wake lock is held and Android 15's daily `dataSync` budget is not spent while waiting.
- 2026-11-04: "Backup complete: N photos" says photos for any file type (as asked); videos count too.
