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
        // Ente's ONNX Runtime build (WebGPU + XNNPACK) for ente-ml, exactly as Ente's photos app gets it
        // (mobile/apps/photos/android/app/build.gradle): a plain GitHub release, so only this group may come from
        // it and its SHA-256 is pinned in gradle/verification-metadata.xml.
        exclusiveContent {
            forRepository {
                ivy {
                    name = "enteOnnxRuntimePackaging"
                    url = uri("https://github.com/ente/ort-packaging/releases/download")
                    patternLayout {
                        artifact("ort-[revision]/[artifact]-[revision].[ext]")
                    }
                    metadataSources { artifact() }
                }
            }
            filter { includeGroup("io.ente.onnxruntime") }
        }
    }
}

rootProject.name = "photovault-native"
include(":app")
include(":core") // photovault-core: UniFFI Kotlin bindings + libphotovault_core.so (built by CI's core job)
// ":baselineprofile" (Baseline Profile + Macrobenchmark) is added by the perf track; keep that name free.
