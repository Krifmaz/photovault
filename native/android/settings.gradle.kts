// Photovault Native (APK 2): the Kotlin / Jetpack Compose client. See README.md.
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

rootProject.name = "photovault-native"
include(":app")
include(":core") // photovault-core: UniFFI Kotlin bindings + libphotovault_core.so (built by CI's core job)
// ":baselineprofile" (Baseline Profile + Macrobenchmark) is added by the perf track; keep that name free.
