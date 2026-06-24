plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
}

android {
    namespace = "com.auroragba"
    compileSdk = 34

    defaultConfig {
        applicationId = "com.auroragba"
        minSdk = 24
        targetSdk = 34
        versionCode = 57
        versionName = "0.57.1"
        // ABIs que empacotamos (emulador x86_64 + dispositivos arm64). As `.so`
        // são geradas pelo cargo-ndk em src/main/jniLibs/<abi>/ (ver android/README.md).
        ndk {
            abiFilters += listOf("arm64-v8a", "x86_64")
        }
    }

    buildTypes {
        release {
            isMinifyEnabled = false
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
    kotlinOptions {
        jvmTarget = "17"
    }
}
