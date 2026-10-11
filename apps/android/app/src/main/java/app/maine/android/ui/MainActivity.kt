package app.maine.android.ui

import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.compose.material3.MaterialTheme
import androidx.lifecycle.viewmodel.compose.viewModel

class MainActivity : ComponentActivity() {

    private var viewModel: MaineViewModel? = null

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()
        setContent {
            MaterialTheme {
                val vm: MaineViewModel = viewModel()
                viewModel = vm
                MaineApp(vm)
            }
        }
    }

    override fun onStop() {
        // Checkpoint session scrollback like a macOS quit; records are kept.
        viewModel?.onBackgrounded()
        super.onStop()
    }
}
