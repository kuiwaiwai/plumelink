plugins {
    id("com.android.application")
}

dependencies {
    implementation("com.journeyapps:zxing-android-embedded:4.3.0")
    // ZXing 4.3.0's published POM omits its AndroidX implementation dependencies.
    implementation("androidx.core:core:1.6.0")
    implementation("androidx.fragment:fragment:1.3.6")
    implementation("com.squareup.okhttp3:okhttp:4.12.0")
    testImplementation("junit:junit:4.13.2")
    testImplementation("org.json:json:20240303")
}

dependencyLocking {
    lockAllConfigurations()
}

val mobileAssetsDir = file("../../tauri-dist/mobile")
val signingValues = listOf("PLUMELINK_KEYSTORE", "PLUMELINK_STORE_PASSWORD", "PLUMELINK_KEY_ALIAS", "PLUMELINK_KEY_PASSWORD")
    .associateWith { System.getenv(it).orEmpty() }
val hasReleaseSigning = signingValues.values.all { it.isNotBlank() }
gradle.taskGraph.whenReady {
    if (allTasks.any { it.project == project && it.name.contains("release", ignoreCase = true) }) {
        check(hasReleaseSigning) { "Release signing requires PLUMELINK_KEYSTORE, PLUMELINK_STORE_PASSWORD, PLUMELINK_KEY_ALIAS and PLUMELINK_KEY_PASSWORD." }
    }
}
val verifyMobileAssets by tasks.registering {
    doLast {
        check(file("$mobileAssetsDir/index.html").isFile) {
            "Mobile UI assets are missing. Run npm run build:android from the repository root."
        }
        check(file("$mobileAssetsDir/assets").listFiles()?.any { it.name.startsWith("qft-") && it.extension == "wasm" } == true) {
            "QFastTransmission WASM is missing. Rebuild the mobile frontend before packaging."
        }
    }
}

tasks.named("preBuild") {
    dependsOn(verifyMobileAssets)
}

android {
    namespace = "com.kuiwaiwai.plumelink"
    compileSdk = 35
    testOptions { unitTests.isReturnDefaultValues = true }
    if (hasReleaseSigning) {
        signingConfigs.create("publisher") {
            storeFile = file(signingValues.getValue("PLUMELINK_KEYSTORE"))
            storePassword = signingValues.getValue("PLUMELINK_STORE_PASSWORD")
            keyAlias = signingValues.getValue("PLUMELINK_KEY_ALIAS")
            keyPassword = signingValues.getValue("PLUMELINK_KEY_PASSWORD")
        }
    }

    defaultConfig {
        applicationId = "com.kuiwaiwai.plumelink"
        minSdk = 26
        targetSdk = 35
        versionCode = 1
        versionName = "0.1.0"
    }

    buildTypes {
        release {
            vcsInfo { include = false }
            if (hasReleaseSigning) signingConfig = signingConfigs.getByName("publisher")
            isMinifyEnabled = true
            proguardFiles(
                getDefaultProguardFile("proguard-android-optimize.txt"),
                "proguard-rules.pro"
            )
        }
    }

    sourceSets {
        getByName("main") {
            assets.srcDir(mobileAssetsDir)
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
}
