import org.jetbrains.kotlin.gradle.dsl.JvmTarget

// Macrobenchmarks for both Photovault APKs. The test APK instruments itself and
// drives whichever app the `targetPackage` instrumentation argument names
// (default app.photovault.native). See ../../README.md for how to run it.
plugins {
    alias(libs.plugins.android.test)
    alias(libs.plugins.kotlin.android)
}

android {
    namespace = "app.photovault.perf.macrobenchmark"
    compileSdk = libs.versions.compileSdk.get().toInt()

    defaultConfig {
        minSdk = libs.versions.minSdk.get().toInt()
        targetSdk = libs.versions.targetSdk.get().toInt()
        testInstrumentationRunner = "androidx.test.runner.AndroidJUnitRunner"
        testInstrumentationRunnerArguments["targetPackage"] = "app.photovault.native"
    }

    buildTypes {
        // Release variant of the test APK, signed with the debug key so it can be
        // installed next to any app build. Both variants work for measuring:
        // Macrobenchmark only requires the *target app* to be non-debuggable.
        getByName("release") {
            isMinifyEnabled = false
            signingConfig = signingConfigs.getByName("debug")
        }
    }

    sourceSets.getByName("main").java.srcDir("../journeys/src/main/kotlin")

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    targetProjectPath = ":target-stub"
    experimentalProperties["android.experimental.self-instrumenting"] = true

    lint {
        abortOnError = true
        checkReleaseBuilds = false
    }
}

kotlin {
    compilerOptions {
        jvmTarget.set(JvmTarget.JVM_17)
    }
}

dependencies {
    implementation(libs.benchmark.macro.junit4)
    implementation(libs.uiautomator)
    implementation(libs.androidx.test.ext.junit)
    implementation(libs.androidx.test.runner)
}
