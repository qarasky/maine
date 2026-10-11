package app.maine.android.ui

import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.BottomSheetDefaults
import androidx.compose.material3.Button
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.unit.dp
import app.maine.android.model.Host

/**
 * New/edit host sheet. Password input is only possible while the vault is
 * unlocked, and a password is required to save — mirroring macOS.
 *
 * Unlike macOS (and the shared schema, which still permits it), a username
 * is required here: this client always dials with an explicit user.
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun HostEditorSheet(
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

    ModalBottomSheet(
        onDismissRequest = onDismiss,
        sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true),
        shape = RoundedCornerShape(topStart = 28.dp, topEnd = 28.dp),
        containerColor = MaterialTheme.colorScheme.surface,
        dragHandle = { BottomSheetDefaults.DragHandle() },
    ) {
        Column(
            modifier = Modifier
                .fillMaxWidth()
                .padding(horizontal = 24.dp)
                .navigationBarsPadding(),
        ) {
            Text(
                if (initial.name.isEmpty() && initial.hostname.isEmpty()) "New host" else "Edit host",
                style = MaterialTheme.typography.titleLarge,
                fontWeight = FontWeight.SemiBold,
            )
            Spacer(Modifier.height(16.dp))
            OutlinedTextField(
                value = name,
                onValueChange = { name = it },
                label = { Text("Name") },
                singleLine = true,
                shape = RoundedCornerShape(16.dp),
                colors = maineFieldColors(),
                modifier = Modifier.fillMaxWidth(),
            )
            Spacer(Modifier.height(8.dp))
            OutlinedTextField(
                value = hostname,
                onValueChange = { hostname = it },
                label = { Text("Hostname or IP") },
                singleLine = true,
                shape = RoundedCornerShape(16.dp),
                colors = maineFieldColors(),
                modifier = Modifier.fillMaxWidth(),
            )
            Spacer(Modifier.height(8.dp))
            OutlinedTextField(
                value = portText,
                onValueChange = { portText = it.filter { c -> c.isDigit() }.take(5) },
                label = { Text("Port") },
                singleLine = true,
                keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Number),
                shape = RoundedCornerShape(16.dp),
                colors = maineFieldColors(),
                modifier = Modifier.fillMaxWidth(),
            )
            Spacer(Modifier.height(8.dp))
            OutlinedTextField(
                value = username,
                onValueChange = { username = it },
                label = { Text("Username") },
                singleLine = true,
                shape = RoundedCornerShape(16.dp),
                colors = maineFieldColors(),
                modifier = Modifier.fillMaxWidth(),
            )
            Spacer(Modifier.height(8.dp))
            OutlinedTextField(
                value = password,
                onValueChange = onPasswordChange,
                label = { Text("Password") },
                visualTransformation = PasswordVisualTransformation(),
                singleLine = true,
                keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Password),
                shape = RoundedCornerShape(16.dp),
                colors = maineFieldColors(),
                modifier = Modifier.fillMaxWidth(),
            )
            val message = localError ?: error
            if (message != null) {
                Spacer(Modifier.height(8.dp))
                Text(message, color = MaterialTheme.colorScheme.error)
            }
            Spacer(Modifier.height(16.dp))
            Button(
                onClick = {
                    if (username.trim().isEmpty()) {
                        localError = "A username is required."
                        return@Button
                    }
                    val port = portText.toIntOrNull()
                    if (port == null) {
                        localError = "The port must be between 1 and 65535."
                        return@Button
                    }
                    localError = null
                    onSave(
                        initial.copy(
                            name = name,
                            hostname = hostname,
                            port = port,
                            username = username.trim(),
                        ),
                    )
                },
                shape = CircleShape,
                modifier = Modifier
                    .fillMaxWidth()
                    .height(56.dp),
            ) {
                Text("Save", style = MaterialTheme.typography.titleMedium)
            }
            TextButton(
                onClick = onDismiss,
                colors = androidx.compose.material3.ButtonDefaults.textButtonColors(
                    contentColor = MaterialTheme.colorScheme.onSurface,
                ),
                modifier = Modifier.align(Alignment.CenterHorizontally),
            ) { Text("Cancel") }
            Spacer(Modifier.height(16.dp))
        }
    }
}
