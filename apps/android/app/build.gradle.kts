plugins {
    alias(libs.plugins.android.application)
    alias(libs.plugins.kotlin.android)
    alias(libs.plugins.kotlin.compose)
    alias(libs.plugins.kotlin.serialization)
}

android {
    namespace = "app.maine.android"
    compileSdk = 36

    defaultConfig {
        applicationId = "app.maine.android"
        minSdk = 28
        targetSdk = 36
        versionCode = 1
        versionName = "1.0.0"
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

    buildFeatures {
        compose = true
    }
}

dependencies {
    implementation(platform(libs.androidx.compose.bom))
    implementation(libs.androidx.ui)
    implementation(libs.androidx.ui.tooling.preview)
    implementation(libs.androidx.material3)
    implementation(libs.androidx.material.icons.core)
    implementation(libs.androidx.activity.compose)
    implementation(libs.androidx.lifecycle.runtime.compose)
    implementation(libs.androidx.lifecycle.viewmodel.compose)
    implementation(libs.kotlinx.coroutines.android)
    implementation(libs.kotlinx.serialization.json)
    // Real terminal emulation (VT/xterm parsing + Canvas rendering), vendored
    // from termux-app under Apache-2.0. See apps/android/terminal.
    implementation(project(":terminal"))
    // SSH transport. Note: sshj pulls in BouncyCastle, and Android already
    // registers a stripped-down provider under the same name ("BC"), so the
    // full one has to be installed at runtime — see installSshSecurityProvider.
    implementation(libs.sshj)
    // Declared explicitly (sshj brings it transitively at runtime only) because
    // that provider swap references BouncyCastleProvider at compile time.
    implementation(libs.bouncycastle.bcprov)

    testImplementation(libs.junit)
}
