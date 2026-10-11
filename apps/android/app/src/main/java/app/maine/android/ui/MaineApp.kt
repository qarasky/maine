package app.maine.android.ui

import androidx.compose.animation.AnimatedContent
import androidx.compose.animation.core.tween
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.slideInVertically
import androidx.compose.animation.togetherWith
import androidx.activity.compose.BackHandler
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.Scaffold
import androidx.compose.runtime.Composable
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import app.maine.android.model.Host

/**
 * Root navigation. Phones navigate host list <-> session; the vault gate
 * sits above everything, exactly like the macOS master-password sheet.
 */
@Composable
fun MaineApp(viewModel: MaineViewModel) {
    val locked = viewModel.locked
    var route by remember { mutableStateOf<Route>(Route.Hosts) }
    val sessions by viewModel.sessions.openSessions.collectAsState()
    val selectedId by viewModel.sessions.selectedSessionId.collectAsState()

    if (locked) {
        route = Route.Hosts
        UnlockScreen(
            vaultExists = viewModel.vaultExists,
            busy = viewModel.unlockBusy,
            error = viewModel.unlockError,
            onUnlock = viewModel::unlock,
        )
        return
    }

    // On a session, Back returns to the host list. Without this the system back
    // gesture finishes the activity and drops the user out of the app.
    BackHandler(enabled = route is Route.Session) {
        route = Route.Hosts
    }

    Scaffold(modifier = Modifier.fillMaxSize()) { inner ->
        AnimatedContent(
            targetState = route,
            transitionSpec = {
                fadeIn(animationSpec = tween(180)) +
                    slideInVertically(
                        animationSpec = tween(200),
                        initialOffsetY = { it / 12 },
                    ) togetherWith
                    fadeOut(animationSpec = tween(100))
            },
            label = "route",
            modifier = Modifier.fillMaxSize(),
        ) { current ->
            when (current) {
                is Route.Hosts -> HostListScreen(
                    modifier = Modifier.padding(inner),
                    viewModel = viewModel,
                    onOpenSession = { route = Route.Session(it) },
                )
                is Route.Session -> {
                    val session = sessions.firstOrNull { it.id == current.sessionId }
                        ?: sessions.firstOrNull { it.id == selectedId }
                    if (session == null) {
                        route = Route.Hosts
                    } else {
                        SessionScreen(
                            // Top inset only: the terminal runs to the bottom
                            // edge of the screen, so this screen does not take
                            // the bottom system-bar inset.
                            modifier = Modifier.padding(top = inner.calculateTopPadding()),
                            viewModel = viewModel,
                            sessionId = session.id,
                            onBack = { route = Route.Hosts },
                        )
                    }
                }
            }
        }
    }

    val editing = viewModel.editingHost
    if (editing != null) {
        HostEditorSheet(
            initial = editing,
            password = viewModel.editorPassword,
            onPasswordChange = { viewModel.editorPassword = it },
            error = viewModel.editorError,
            onDismiss = viewModel::cancelEdit,
            onSave = { viewModel.commitEdit(it) },
        )
    }
}

sealed class Route {
    data object Hosts : Route()
    data class Session(val sessionId: String) : Route()
}

/** Blank starting point for the "New host" editor. */
fun blankHost(): Host = Host(name = "", hostname = "")
