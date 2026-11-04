// Empty placeholder app. It exists only because com.android.test requires a
// targetProjectPath; nothing in it is ever installed for measurement.
plugins {
    alias(libs.plugins.android.application)
}

android {
    namespace = "app.photovault.perf.stub"
    compileSdk = libs.versions.compileSdk.get().toInt()

    defaultConfig {
        applicationId = "app.photovault.perf.stub"
        minSdk = libs.versions.minSdk.get().toInt()
        targetSdk = libs.versions.targetSdk.get().toInt()
        versionCode = 1
        versionName = "1"
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    lint {
        abortOnError = true
        checkReleaseBuilds = false
    }
}
