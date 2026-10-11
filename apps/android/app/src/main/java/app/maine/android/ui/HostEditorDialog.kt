package app.maine.android.ui

import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.unit.dp
import app.maine.android.model.Host

/**
 * New/edit host dialog. Password input is only possible while the vault is
 * unlocked, and a password is required to save — mirroring macOS.
 */
@Composable
fun HostEditorDialog(
    initial: Host,
    password: String,
    onPasswordChange: (String) -> Unit,
    error: String?,
    onDismiss: () -> Unit,
    onSave: (Host) -> Boolean,
) {
    var name by remember(initial.id) { mutableStateOf(initial.name) }
    var hostname by remember(initial.id) { mutableStateOf(initial.hostname) }
    var portText by remember(initial.id) { mutableStateOf(initial.port.toString()) }
    var username by remember(initial.id) { mutableStateOf(initial.username) }
    var localError by remember(initial.id) { mutableStateOf<String?>(null) }

    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text(if (initial.name.isEmpty() && initial.hostname.isEmpty()) "New host" else "Edit host") },
        text = {
            Column {
                OutlinedTextField(
                    value = name,
                    onValueChange = { name = it },
                    label = { Text("Name") },
                    singleLine = true,
                    modifier = Modifier.fillMaxWidth(),
                )
                Spacer(Modifier.height(8.dp))
                OutlinedTextField(
                    value = hostname,
                    onValueChange = { hostname = it },
                    label = { Text("Hostname or IP") },
                    singleLine = true,
                    modifier = Modifier.fillMaxWidth(),
                )
                Spacer(Modifier.height(8.dp))
                OutlinedTextField(
                    value = portText,
                    onValueChange = { portText = it.filter { c -> c.isDigit() }.take(5) },
                    label = { Text("Port (22 = default)") },
                    singleLine = true,
                    keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Number),
                    modifier = Modifier.fillMaxWidth(),
                )
                Spacer(Modifier.height(8.dp))
                OutlinedTextField(
                    value = username,
                    onValueChange = { username = it },
                    label = { Text("Username (optional)") },
                    singleLine = true,
                    modifier = Modifier.fillMaxWidth(),
                )
                Spacer(Modifier.height(8.dp))
                OutlinedTextField(
                    value = password,
                    onValueChange = onPasswordChange,
                    label = { Text("Password") },
                    visualTransformation = PasswordVisualTransformation(),
                    singleLine = true,
                    modifier = Modifier.fillMaxWidth(),
                )
                val message = localError ?: error
                if (message != null) {
                    Spacer(Modifier.height(8.dp))
                    Text(message, color = MaterialTheme.colorScheme.error)
                }
            }
        },
        confirmButton = {
            TextButton(onClick = {
                val port = portText.toIntOrNull()
                if (port == null) {
                    localError = "The port must be between 1 and 65535."
                    return@TextButton
                }
                localError = null
                onSave(initial.copy(name = name, hostname = hostname, port = port, username = username))
            }) {
                Text("Save")
            }
        },
        dismissButton = {
            TextButton(onClick = onDismiss) { Text("Cancel") }
        },
    )
}
