package app.maine.android.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.ArrowBack
import androidx.compose.material.icons.filled.Close
import androidx.compose.material.icons.filled.Refresh
import androidx.compose.material.icons.filled.Send
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.unit.dp
import app.maine.android.session.TerminalSession
import app.maine.android.ssh.SshTarget

/**
 * One open session: compact connection header, error banner when the
 * transport failed, scrollback kept as plain text, and an input row.
 *
 * This is the stub terminal for the scaffold pass: scrollback plus input,
 * backed by [TerminalSession]. Full terminal emulation (Termux
 * terminal-view) replaces this screen's output view in a later pass; the
 * session lifecycle underneath does not change.
 */
@Composable
fun SessionScreen(
    modifier: Modifier = Modifier,
    viewModel: MaineViewModel,
    sessionId: String,
    onBack: () -> Unit,
) {
    val sessions by viewModel.sessions.openSessions.collectAsState()
    val session = sessions.firstOrNull { it.id == sessionId } ?: return
    val state by session.state.collectAsState()
    val output by session.output.collectAsState()
    val scroll = rememberScrollState()

    LaunchedEffect(output) {
        scroll.animateScrollTo(scroll.maxValue)
    }

    val target = SshTarget.make(host = session.host)

    Column(modifier = modifier.fillMaxSize()) {
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .padding(horizontal = 4.dp, vertical = 4.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            IconButton(onClick = onBack) {
                Icon(Icons.Filled.ArrowBack, contentDescription = "Back to hosts")
            }
            StatusDot(connected = state.isLive)
            Column(modifier = Modifier.weight(1f).padding(horizontal = 8.dp)) {
                Text(session.host.subtitle, style = MaterialTheme.typography.bodyLarge)
                Text(state.label, style = MaterialTheme.typography.bodySmall)
            }
            IconButton(onClick = { viewModel.sessions.reconnectSession(session.id) }) {
                Icon(Icons.Filled.Refresh, contentDescription = "Reconnect")
            }
            IconButton(onClick = {
                viewModel.sessions.closeSession(session.id)
                onBack()
            }) {
                Icon(Icons.Filled.Close, contentDescription = "Close session")
            }
        }

        SelectionContainer {
            Text(
                target.commandLine,
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                modifier = Modifier.padding(horizontal = 16.dp),
            )
        }

        if (state.showsErrorBanner(session.wasIntentionalDisconnect())) {
            val message = when (state) {
                is TerminalSession.State.Failed ->
                    (state as TerminalSession.State.Failed).message
                is TerminalSession.State.Closed ->
                    "Connection closed (exit ${(state as TerminalSession.State.Closed).exitCode ?: "?"})"
                is TerminalSession.State.Disconnected ->
                    "Disconnected (${(state as TerminalSession.State.Disconnected).exitCode ?: "?"})"
                else -> "Connection ended"
            }
            Text(
                message,
                color = MaterialTheme.colorScheme.error,
                modifier = Modifier.padding(horizontal = 16.dp, vertical = 4.dp),
            )
        }

        SelectionContainer(modifier = Modifier.weight(1f)) {
            Text(
                output.ifEmpty { "—" },
                style = MaterialTheme.typography.bodyMedium,
                modifier = Modifier
                    .fillMaxSize()
                    .verticalScroll(scroll)
                    .padding(16.dp),
            )
        }

        var input by remember(session.id) { mutableStateOf("") }
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .padding(8.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            OutlinedTextField(
                value = input,
                onValueChange = { input = it },
                label = { Text("Send input") },
                singleLine = true,
                keyboardOptions = KeyboardOptions(imeAction = ImeAction.Send),
                keyboardActions = KeyboardActions(onSend = {
                    session.send(input + "\n")
                    input = ""
                }),
                enabled = state.isLive,
                modifier = Modifier.weight(1f),
            )
            IconButton(
                onClick = {
                    session.send(input + "\n")
                    input = ""
                },
                enabled = state.isLive,
            ) {
                Icon(Icons.Filled.Send, contentDescription = "Send")
            }
        }
    }
}
