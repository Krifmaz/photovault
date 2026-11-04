plugins {
    alias(libs.plugins.android.application)
    alias(libs.plugins.kotlin.compose)
    alias(libs.plugins.kotlin.serialization)
}

// contracts/apps.json "photovault-native"
val defaultServer: String = providers.gradleProperty("photovault.defaultServer").orElse("").get().trim()
val pvVersionName: String = providers.gradleProperty("photovault.versionName").orElse("0.1.0-beta").get()
val pvVersionCode: Int = providers.gradleProperty("photovault.versionCode").orElse("1").get().toInt()

// The real photovault-core bindings exist only after CI's `core` job (or a local cargo-ndk build) has put
// them in core/build/generated/uniffi. Without them the app is built against FakeCore.
val uniffiDir: File = rootProject.file("core/build/generated/uniffi")
val hasRealCore: Boolean = uniffiDir.isDirectory && uniffiDir.walkTopDown().any { it.isFile && it.extension == "kt" }
logger.lifecycle("Photovault Native: building with ${if (hasRealCore) "photovault-core (UniFFI)" else "FakeCore (no core bindings found)"}")
// CI sets this when the `core` job ran, so a missing artifact fails the build instead of shipping the demo.
if (!hasRealCore && providers.gradleProperty("photovault.requireRealCore").orNull == "true") {
    throw GradleException("photovault.requireRealCore is set but $uniffiDir has no UniFFI bindings")
}
// -Pphotovault.demo=true: use the in-memory demo library even when the real core is built in.
val forceDemo: Boolean = providers.gradleProperty("photovault.demo").orNull == "true"

// Release signing from the environment (set by .github/workflows/android-native.yml). Without a keystore the
// release build is signed with the debug key so local builds still work.
val keystorePath: String? = System.getenv("PV_KEYSTORE")?.takeIf { it.isNotBlank() }
val hasReleaseKey: Boolean = keystorePath?.let { file(it).isFile } == true

android {
    namespace = "app.photovault.android"
    compileSdk = 37

    defaultConfig {
        applicationId = "app.photovault.native"
        minSdk = 28
        targetSdk = 36
        versionCode = pvVersionCode
        versionName = pvVersionName
        buildConfigField("String", "DEFAULT_SERVER_URL", "\"${defaultServer.replace("\\", "\\\\").replace("\"", "\\\"")}\"")
        buildConfigField("boolean", "REAL_CORE", hasRealCore.toString())
        buildConfigField("boolean", "DEMO", forceDemo.toString())
        ndk {
            abiFilters += listOf("arm64-v8a", "x86_64")
        }
    }

    signingConfigs {
        create("release") {
            if (hasReleaseKey) {
                storeFile = file(keystorePath!!)
                storePassword = System.getenv("PV_KEYSTORE_PASSWORD")
                keyAlias = System.getenv("PV_KEY_ALIAS")
                keyPassword = System.getenv("PV_KEY_PASSWORD")
            }
        }
    }

    buildTypes {
        getByName("release") {
            isMinifyEnabled = true
            isShrinkResources = true
            proguardFiles(getDefaultProguardFile("proguard-android-optimize.txt"), "proguard-rules.pro")
            signingConfig = if (hasReleaseKey) signingConfigs.getByName("release") else signingConfigs.getByName("debug")
        }
        getByName("debug") {
            applicationIdSuffix = ".debug"
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    buildFeatures {
        compose = true
        buildConfig = true
    }

    sourceSets {
        getByName("main") {
            // exactly one CoreProvider: the UniFFI adapter or the in-memory fake
            kotlin.directories += file(if (hasRealCore) "src/realCore/kotlin" else "src/fakeCore/kotlin").path
        }
    }

    testOptions {
        unitTests.isReturnDefaultValues = true
    }

    lint {
        // Release lint must pass in CI; warnings are reported, not fatal.
        abortOnError = true
        checkReleaseBuilds = true
    }

    packaging {
        resources {
            excludes += "/META-INF/{AL2.0,LGPL2.1}"
        }
    }
}

dependencies {
    implementation(project(":core"))

    implementation(platform(libs.androidx.compose.bom))
    implementation(libs.androidx.compose.ui)
    implementation(libs.androidx.compose.ui.graphics)
    implementation(libs.androidx.compose.foundation)
    implementation(libs.androidx.compose.animation)
    implementation(libs.androidx.compose.material3)
    implementation(libs.androidx.activity.compose)
    implementation(libs.androidx.lifecycle.runtime.compose)
    implementation(libs.androidx.lifecycle.viewmodel.compose)
    implementation(libs.androidx.navigation.compose)
    implementation(libs.kotlinx.serialization.json)
    implementation(libs.kotlinx.coroutines.android)

    implementation(libs.coil.compose)
    implementation(libs.coil.core)

    implementation(libs.androidx.media3.exoplayer)
    implementation(libs.androidx.media3.exoplayer.hls)
    implementation(libs.androidx.media3.ui)
    implementation(libs.androidx.media3.ui.compose)

    implementation(libs.androidx.camera.core)
    implementation(libs.androidx.camera.camera2)
    implementation(libs.androidx.camera.lifecycle)
    implementation(libs.androidx.camera.view)
    implementation(libs.zxing.core)

    implementation(libs.androidx.profileinstaller)

    testImplementation(libs.junit)
    testImplementation(libs.kotlinx.coroutines.test)
}
