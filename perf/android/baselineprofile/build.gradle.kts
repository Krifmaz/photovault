import org.jetbrains.kotlin.gradle.dsl.JvmTarget

// Baseline Profile + startup profile generator for Photovault Native
// (app.photovault.native). Run it with ../../scripts/collect-baseline-profile.sh,
// which installs this APK, runs the generator and copies the result into
// native/android/app/src/main/.
plugins {
    alias(libs.plugins.android.test)
    alias(libs.plugins.kotlin.android)
}

android {
    namespace = "app.photovault.perf.baselineprofile"
    compileSdk = libs.versions.compileSdk.get().toInt()

    defaultConfig {
        minSdk = libs.versions.minSdk.get().toInt()
        targetSdk = libs.versions.targetSdk.get().toInt()
        testInstrumentationRunner = "androidx.test.runner.AndroidJUnitRunner"
        testInstrumentationRunnerArguments["targetPackage"] = "app.photovault.native"
    }

    buildTypes {
        create("release") {
            matchingFallbacks += listOf("release")
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
