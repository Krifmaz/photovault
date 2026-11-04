package app.photovault.perf.journeys

import android.os.Bundle
import androidx.benchmark.macro.BaselineProfileMode
import androidx.benchmark.macro.CompilationMode
import androidx.test.platform.app.InstrumentationRegistry

/** Which UI toolkit the app under test is built with. Selectors and metrics differ. */
enum class AppKind { NATIVE, FLUTTER }

/**
 * The app under test, read from instrumentation arguments:
 *
 * - `targetPackage`  package to drive. Default `app.photovault.native`.
 *                    `app.photovault` (or `app.photovault.debug`) is the Flutter app.
 * - `appKind`        `native` or `flutter`. Default: derived from the package name.
 * - `iterations`     iterations per benchmark. Default 10.
 * - `compilation`    `none`, `partial` (Baseline Profile if the APK has one) or `full`.
 *                    Default `partial`, which is what a store/F-Droid style install sees
 *                    once profileinstaller has run.
 * - `requireTimeline` `false` for a smoke run on a device whose app is not signed in
 *                    (CI emulator): startup is measured to the first frame only and the
 *                    scroll/viewer benchmarks are skipped. Default `true`.
 * - `flutterTrace`   `true` to launch the Flutter app with the engine's `trace-systrace`
 *                    intent extra and record its frame sections. Only works with a
 *                    `--profile` Flutter build; release builds compile the timeline out.
 */
object TargetApp {
    const val NATIVE_PACKAGE = "app.photovault.native"
    const val FLUTTER_PACKAGE = "app.photovault"

    private val args: Bundle
        get() = InstrumentationRegistry.getArguments()

    val packageName: String
        get() = args.getString("targetPackage")?.takeIf { it.isNotBlank() } ?: NATIVE_PACKAGE

    val kind: AppKind
        get() = when (args.getString("appKind")?.lowercase()) {
            "native" -> AppKind.NATIVE
            "flutter" -> AppKind.FLUTTER
            else -> if (packageName.startsWith(NATIVE_PACKAGE)) AppKind.NATIVE else AppKind.FLUTTER
        }

    val iterations: Int
        get() = args.getString("iterations")?.toIntOrNull()?.coerceAtLeast(1) ?: 10

    val flutterTrace: Boolean
        get() = args.getString("flutterTrace").toBoolean()

    val requireTimeline: Boolean
        get() = args.getString("requireTimeline")?.toBooleanStrictOrNull() ?: true

    val compilationMode: CompilationMode
        get() = when (args.getString("compilation")?.lowercase()) {
            "none" -> CompilationMode.None()
            "full" -> CompilationMode.Full()
            else -> CompilationMode.Partial(baselineProfileMode = BaselineProfileMode.UseIfAvailable)
        }
}
