package app.maine.android.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.Button
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.unit.dp

/**
 * The master-password gate shown on every launch. First run creates the
 * vault (12+ characters); afterwards it only unlocks. Canceling is leaving
 * the app: passwords stay locked either way.
 */
@Composable
fun UnlockScreen(
    vaultExists: Boolean,
    busy: Boolean,
    error: String?,
    onUnlock: (String) -> Unit,
) {
    var password by remember { mutableStateOf("") }
    Column(
        modifier = Modifier
            .fillMaxSize()
            .padding(24.dp),
        verticalArrangement = Arrangement.Center,
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        Text("Maine", style = MaterialTheme.typography.headlineMedium)
        Spacer(Modifier.height(8.dp))
        Text(
            if (vaultExists) "Enter the master password to unlock hosts."
            else "Create a master password of at least 12 characters.",
            style = MaterialTheme.typography.bodyMedium,
        )
        Spacer(Modifier.height(16.dp))
        OutlinedTextField(
            value = password,
            onValueChange = { password = it },
            label = { Text("Master password") },
            visualTransformation = PasswordVisualTransformation(),
            singleLine = true,
            keyboardOptions = KeyboardOptions(imeAction = ImeAction.Go),
            keyboardActions = KeyboardActions(onGo = { onUnlock(password) }),
            isError = error != null,
            modifier = Modifier.fillMaxWidth(),
        )
        if (error != null) {
            Spacer(Modifier.height(8.dp))
            Text(error, color = MaterialTheme.colorScheme.error)
        }
        Spacer(Modifier.height(16.dp))
        Button(
            onClick = { onUnlock(password) },
            enabled = !busy,
            modifier = Modifier.fillMaxWidth(),
        ) {
            Text(
                when {
                    busy && vaultExists -> "Unlocking…"
                    busy -> "Creating…"
                    vaultExists -> "Unlock"
                    else -> "Create vault"
                },
            )
        }
    }
}
