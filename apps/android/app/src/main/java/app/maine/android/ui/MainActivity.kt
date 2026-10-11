package app.maine.android.ui

import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.ui.Modifier
import androidx.lifecycle.viewmodel.compose.viewModel
import app.maine.android.session.installSshSecurityProvider

class MainActivity : ComponentActivity() {

    private var viewModel: MaineViewModel? = null

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        // Both must be in place before anything can connect or render.
        installSshSecurityProvider()
        applyMaineTerminalPalette()
        enableEdgeToEdge()
        setContent {
            MaineTheme {
                // Surface sets the content color (onBackground). Without it,
                // Text falls back to black — invisible on the dark theme.
                Surface(
                    modifier = Modifier.fillMaxSize(),
                    color = MaterialTheme.colorScheme.background,
                ) {
                    val vm: MaineViewModel = viewModel()
                    viewModel = vm
                    MaineApp(vm)
                }
            }
        }
    }

    override fun onStop() {
        // Checkpoint session scrollback like a macOS quit; records are kept.
        viewModel?.onBackgrounded()
        super.onStop()
    }
}
