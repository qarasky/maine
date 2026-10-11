// Vendored terminal emulation, not Maine's own code.
//
// Sources are copied verbatim from termux/termux-app (Apache-2.0) so the Android
// client can render a real VT/xterm terminal instead of a plain text view:
//
//   terminal-emulator/src/main/java/com/termux/terminal  (VT parser + buffers)
//   terminal-view/src/main/java/com/termux/view          (Canvas renderer + view)
//
// The only local change is in TerminalSession.java: an added `Transport` path so
// an SSH byte stream can drive the emulator in place of Termux's native PTY
// fork. See THIRD_PARTY_LICENSES.md for provenance and the Apache-2.0 notice.
//
// Namespace is `com.termux.view` because the text-selection classes import
// `com.termux.view.R` (upstream's terminal-view namespace). Keeping it also
// keeps the resource namespace identical to upstream, so re-vendoring is a
// straight directory copy plus the documented Transport patch.
plugins {
    alias(libs.plugins.android.library)
}

android {
    namespace = "com.termux.view"
    compileSdk = 36

    defaultConfig {
        minSdk = 28
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
}

dependencies {
    implementation(libs.androidx.annotation)
}
