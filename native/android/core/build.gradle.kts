// photovault-core for Android: the UniFFI-generated Kotlin (package app.photovault.core) and
// libphotovault_core.so for arm64-v8a / x86_64. Both are produced by the `core` job of
// .github/workflows/android-native.yml (cargo-ndk + uniffi-bindgen) into
//   core/build/generated/uniffi   and   core/build/jniLibs/<abi>/libphotovault_core.so
// When they are absent this module is just JNA + coroutines, and :app falls back to its FakeCore.
plugins {
    alias(libs.plugins.android.library)
}

val uniffiDir = layout.projectDirectory.dir("build/generated/uniffi").asFile
val jniLibsDir = layout.projectDirectory.dir("build/jniLibs").asFile

android {
    namespace = "app.photovault.core"
    compileSdk = 37

    defaultConfig {
        minSdk = 28
        consumerProguardFiles("consumer-rules.pro")
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    lint {
        // This module holds only UniFFI-generated code. It picks java.lang.ref.Cleaner (API 33) by
        // reflection and falls back to JNA's cleaner below that, which NewApi cannot see.
        disable += "NewApi"
    }

    sourceSets {
        getByName("main") {
            kotlin.directories += uniffiDir.path
            jniLibs.directories += jniLibsDir.path
        }
    }
}

dependencies {
    // UniFFI's Kotlin bindings load the library through JNA; the aar carries libjnidispatch.so per ABI.
    api("${libs.jna.get().module}:${libs.versions.jna.get()}@aar")
    api(libs.kotlinx.coroutines.core)
}
