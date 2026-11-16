// PDF26: PdfCraft for Android tablets, as a handwriting notes app. The Rust library
// (libpdfcraft_android.so) is built first with cargo-ndk
// into src/main/jniLibs; see apps/pdfcraft-android/README.md.
plugins {
    id("com.android.application")
}

android {
    namespace = "ai.storyteller.pdfcraft"
    compileSdk = 35
    ndkVersion = "27.3.13750724"

    defaultConfig {
        applicationId = "ai.storyteller.pdfcraft"
        minSdk = 26
        targetSdk = 35
        versionCode = 2101
        versionName = "0.21.1"
    }

    buildTypes {
        release {
            isMinifyEnabled = false
            // Unsigned by default; sign with your own key (README.md).
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    packaging {
        // Compressed: the library is large, and a smaller APK is easier to copy to a phone.
        jniLibs.useLegacyPackaging = true
    }
}

// The launcher icon is PDF26's own adaptive vector icon (src/main/res/drawable/ic_launcher_*.xml,
// mipmap-anydpi-v26), drawn for this app and attributed in ATTRIBUTION.toml.

dependencies {
    // GameActivity: the activity android-activity's game-activity backend drives (Apache-2.0).
    implementation("androidx.games:games-activity:4.4.0")
    implementation("androidx.appcompat:appcompat:1.7.0")
    implementation("androidx.core:core:1.13.1")
    // games-activity and appcompat pull different Kotlin standard libraries; one platform aligns them.
    implementation(platform("org.jetbrains.kotlin:kotlin-bom:1.8.22"))
}
