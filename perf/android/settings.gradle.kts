// Standalone perf project. It builds test APKs only; the apps under test are
// installed separately and addressed by package name (see ../README.md).
pluginManagement {
    repositories {
        google()
        mavenCentral()
        gradlePluginPortal()
    }
}

dependencyResolutionManagement {
    repositoriesMode.set(RepositoriesMode.FAIL_ON_PROJECT_REPOS)
    repositories {
        google()
        mavenCentral()
    }
}

rootProject.name = "photovault-perf"

// com.android.test modules must name a target app project. `target-stub` is an
// empty placeholder: the test APKs are self-instrumenting and drive the real app
// (instrumentation arg `targetPackage`) through UiAutomator and the shell.
include(":target-stub")
include(":macrobenchmark")
include(":baselineprofile")
