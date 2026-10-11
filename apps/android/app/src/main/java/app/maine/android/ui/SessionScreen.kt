package app.maine.android.ui

import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.asPaddingValues
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.ime
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.navigationBars
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.ArrowBack
import androidx.compose.material.icons.filled.Close
import androidx.compose.material.icons.filled.Refresh
import androidx.compose.material3.Card
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import app.maine.android.session.TerminalSession

/**
 * One open session: a compact connection header, an error banner when the
 * transport failed, and the terminal itself below.
 *
 * The terminal is a real emulator (see [TerminalSurface]) and is also the input
 * surface — tapping it raises the keyboard and typing goes to the remote shell.
 * There is no separate input field.
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

    val extraKeys = remember(session.id) { ExtraKeysState() }
    val density = LocalDensity.current
    // Read before `imePadding()` consumes the insets below: the keys row only
    // belongs on screen when the soft keyboard is, like Termux.
    val imeVisible = WindowInsets.ime.getBottom(density) > 0
    val gestureInset = WindowInsets.navigationBars.asPaddingValues().calculateBottomPadding()

    Column(modifier = modifier.fillMaxSize().imePadding()) {
        Card(
            shape = RoundedCornerShape(24.dp),
            colors = CardDefaults.cardColors(containerColor = MaterialTheme.colorScheme.surface),
            border = BorderStroke(1.dp, MaterialTheme.colorScheme.outlineVariant),
            modifier = Modifier
                .fillMaxWidth()
                .padding(horizontal = 16.dp, vertical = 8.dp),
        ) {
            Row(
                modifier = Modifier
                    .fillMaxWidth()
                    .padding(horizontal = 8.dp, vertical = 8.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                IconButton(
                    onClick = onBack,
                    modifier = Modifier.size(40.dp),
                ) {
                    Icon(Icons.Filled.ArrowBack, contentDescription = "Back to hosts")
                }
                StatusDot(kind = state.statusKind())
                Column(modifier = Modifier.weight(1f).padding(horizontal = 12.dp)) {
                    Text(
                        session.host.displayName,
                        style = MaterialTheme.typography.bodyLarge,
                        fontWeight = FontWeight.SemiBold,
                    )
                    Text(
                        state.label.uppercase(),
                        fontFamily = MaineMono,
                        fontSize = 11.sp,
                        letterSpacing = 1.sp,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
                IconButton(
                    onClick = { viewModel.sessions.reconnectSession(session.id) },
                    modifier = Modifier.size(40.dp),
                ) {
                    Icon(Icons.Filled.Refresh, contentDescription = "Reconnect")
                }
                IconButton(
                    onClick = {
                        viewModel.sessions.closeSession(session.id)
                        onBack()
                    },
                    modifier = Modifier.size(40.dp),
                ) {
                    Icon(Icons.Filled.Close, contentDescription = "Close session")
                }
            }
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
            Card(
                shape = RoundedCornerShape(16.dp),
                colors = CardDefaults.cardColors(
                    containerColor = MaterialTheme.colorScheme.errorContainer,
                    contentColor = MaterialTheme.colorScheme.onErrorContainer,
                ),
                modifier = Modifier
                    .fillMaxWidth()
                    .padding(horizontal = 16.dp),
            ) {
                Text(message, modifier = Modifier.padding(12.dp))
            }
            Spacer(Modifier.height(8.dp))
        }

        // Full-bleed: the terminal sheet reaches the left, right and bottom
        // screen edges, with only its top corners rounded, so it reads as a
        // sheet tucked under the header. The keys row is part of that sheet and
        // sits directly above the soft keyboard.
        Column(
            modifier = Modifier
                .weight(1f)
                .fillMaxWidth()
                .clip(RoundedCornerShape(topStart = 20.dp, topEnd = 20.dp))
                .background(MaineColors.Terminal),
        ) {
            TerminalSurface(
                session = session,
                extraKeys = extraKeys,
                modifier = Modifier
                    .weight(1f)
                    .fillMaxWidth()
                    .padding(
                        start = TerminalInset,
                        end = TerminalInset,
                        top = TerminalInset,
                        // The keys row already holds the bottom clear of the
                        // gesture bar; only pad for it when the row is absent.
                        bottom = if (imeVisible) TerminalInset else TerminalInset + gestureInset,
                    ),
            )
            if (imeVisible) {
                ExtraKeysRow(
                    state = extraKeys,
                    onKey = { key ->
                        session.sendSpecialKey(key.keyCode, extraKeys.modifierMask())
                        extraKeys.clear()
                    },
                )
            }
        }
    }
}
