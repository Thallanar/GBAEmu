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
        versionCode = 59
        versionName = "0.59.2"
        // ABIs que empacotamos (emulador x86_64 + dispositivos arm64). As `.so`
        // são geradas pelo cargo-ndk em src/main/jniLibs/<abi>/ (ver android/README.md).
        ndk {
            abiFilters += listOf("arm64-v8a", "x86_64")
        }
    }

    // Empacota os shaders compartilhados (fonte canônica única na raiz do repo,
    // a mesma que o desktop embute via include_str!). Os `.frag` ficam na raiz
    // dos assets do APK; ver GbaRenderer.loadShaderBody.
    sourceSets {
        getByName("main") {
            assets.srcDir("../../assets/shaders")
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
