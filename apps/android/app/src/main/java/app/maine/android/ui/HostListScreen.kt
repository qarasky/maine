package app.maine.android.ui

import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.core.tween
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.slideInVertically
import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.Image
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Add
import androidx.compose.material.icons.filled.Close
import androidx.compose.material.icons.filled.Delete
import androidx.compose.material.icons.filled.Edit
import androidx.compose.material.icons.filled.Lock
import androidx.compose.material3.Card
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.HorizontalDivider
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
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import app.maine.android.R
import app.maine.android.model.Host
import app.maine.android.session.TerminalSession

/**
 * The host list. A single tap connects immediately: there is no host-details
 * page. Open sessions appear as indented child rows with their own close
 * control, mirroring the macOS sidebar.
 */
@Composable
fun HostListScreen(
    modifier: Modifier = Modifier,
    viewModel: MaineViewModel,
    onOpenSession: (String) -> Unit,
) {
    val hosts by viewModel.store.hosts.collectAsState()
    val sessions by viewModel.sessions.openSessions.collectAsState()
    val selectedId by viewModel.sessions.selectedSessionId.collectAsState()
    val loadFailure by viewModel.store.loadFailure.collectAsState()
    val saveFailure by viewModel.store.saveFailure.collectAsState()

    val query = viewModel.searchQuery.trim()
    val visible = if (query.isEmpty()) hosts
    else hosts.filter {
        it.displayName.contains(query, ignoreCase = true) ||
            it.hostname.contains(query, ignoreCase = true)
    }

    Column(modifier = modifier.fillMaxSize()) {
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .padding(horizontal = 16.dp, vertical = 8.dp),
            horizontalArrangement = Arrangement.SpaceBetween,
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Image(
                    painter = painterResource(R.drawable.maine_mark),
                    contentDescription = "Maine",
                    modifier = Modifier.size(20.dp),
                )
                Spacer(Modifier.width(8.dp))
                Text(
                    "Maine",
                    style = MaterialTheme.typography.titleLarge,
                    fontWeight = FontWeight.SemiBold,
                )
            }
            Row {
                IconButton(onClick = { viewModel.beginNewHost() }) {
                    Icon(Icons.Filled.Add, contentDescription = "New host")
                }
                IconButton(onClick = { viewModel.lock() }) {
                    Icon(Icons.Filled.Lock, contentDescription = "Lock password vault")
                }
            }
        }

        OutlinedTextField(
            value = viewModel.searchQuery,
            onValueChange = { viewModel.searchQuery = it },
            label = { Text("Search hosts") },
            singleLine = true,
            shape = RoundedCornerShape(24.dp),
            colors = maineFieldColors(),
            modifier = Modifier
                .fillMaxWidth()
                .padding(horizontal = 16.dp),
        )

        loadFailure?.let {
            Text(
                it.message,
                color = MaterialTheme.colorScheme.error,
                modifier = Modifier.padding(16.dp),
            )
        }
        saveFailure?.let {
            Text(
                it,
                color = MaterialTheme.colorScheme.error,
                modifier = Modifier.padding(16.dp),
            )
        }

        LazyColumn(
            modifier = Modifier.fillMaxSize(),
            verticalArrangement = Arrangement.spacedBy(10.dp),
        ) {
            item { Spacer(Modifier.height(2.dp)) }
            itemsIndexed(visible, key = { _, host -> host.id }) { index, host ->
                var shown by remember(host.id) { mutableStateOf(false) }
                LaunchedEffect(host.id) { shown = true }
                AnimatedVisibility(
                    visible = shown,
                    enter = fadeIn(animationSpec = tween(180, delayMillis = (index * 35).coerceAtMost(280))) +
                        slideInVertically(
                            animationSpec = tween(200, delayMillis = (index * 35).coerceAtMost(280)),
                            initialOffsetY = { it / 6 },
                        ),
                    exit = fadeOut(animationSpec = tween(100)),
                ) {
                    val mine = sessions.filter { Host.idsEqual(it.host.id, host.id) }
                    HostCard(
                        host = host,
                        sessions = mine,
                        selectedId = selectedId,
                        managerLabel = { viewModel.sessions.label(it) },
                        onOpen = {
                            viewModel.sessions.open(host)
                            viewModel.sessions.sessionFor(host.id)?.let { onOpenSession(it.id) }
                        },
                        onNewSession = {
                            viewModel.sessions.newSession(host)
                            viewModel.sessions.sessionsFor(host.id).lastOrNull()
                                ?.let { onOpenSession(it.id) }
                        },
                        onSelectSession = {
                            viewModel.sessions.selectSession(it)
                            onOpenSession(it)
                        },
                        onCloseSession = { viewModel.sessions.closeSession(it) },
                        onEdit = { viewModel.beginEditHost(host) },
                        onDelete = { viewModel.deleteHost(host) },
                    )
                }
            }
            item { Spacer(Modifier.height(8.dp)) }
        }
    }
}

@Composable
private fun HostCard(
    host: Host,
    sessions: List<TerminalSession>,
    selectedId: String?,
    managerLabel: (TerminalSession) -> String,
    onOpen: () -> Unit,
    onNewSession: () -> Unit,
    onSelectSession: (String) -> Unit,
    onCloseSession: (String) -> Unit,
    onEdit: () -> Unit,
    onDelete: () -> Unit,
) {
    val states = sessions.map { it.state.collectAsState().value }
    val kind = when {
        states.any { it is TerminalSession.State.Failed } -> StatusKind.Failed
        states.any { it is TerminalSession.State.Connecting } -> StatusKind.Connecting
        states.any { it.isLive } -> StatusKind.Live
        else -> StatusKind.Idle
    }

    Card(
        shape = RoundedCornerShape(24.dp),
        colors = CardDefaults.cardColors(containerColor = MaterialTheme.colorScheme.surface),
        border = BorderStroke(1.dp, MaterialTheme.colorScheme.outlineVariant),
        modifier = Modifier
            .fillMaxWidth()
            .padding(horizontal = 16.dp),
    ) {
        Column(modifier = Modifier.fillMaxWidth()) {
            Row(
                modifier = Modifier
                    .fillMaxWidth()
                    .clickable(onClick = onOpen)
                    .padding(horizontal = 16.dp, vertical = 12.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                StatusDot(kind = kind)
                Column(modifier = Modifier.weight(1f).padding(horizontal = 12.dp)) {
                    Text(
                        host.displayName,
                        style = MaterialTheme.typography.bodyLarge,
                        fontWeight = FontWeight.SemiBold,
                    )
                    Text(
                        host.subtitle,
                        fontFamily = MaineMono,
                        fontSize = 12.sp,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
                IconButton(
                    onClick = onNewSession,
                    modifier = Modifier.size(40.dp),
                ) {
                    Icon(Icons.Filled.Add, contentDescription = "New session")
                }
                IconButton(
                    onClick = onEdit,
                    modifier = Modifier.size(40.dp),
                ) {
                    Icon(Icons.Filled.Edit, contentDescription = "Edit host")
                }
                IconButton(
                    onClick = onDelete,
                    modifier = Modifier.size(40.dp),
                ) {
                    Icon(Icons.Filled.Delete, contentDescription = "Delete host")
                }
            }
            sessions.forEachIndexed { index, session ->
                HorizontalDivider(color = MaterialTheme.colorScheme.outlineVariant)
                SessionChildRow(
                    label = managerLabel(session),
                    state = states[index],
                    selected = session.id == selectedId,
                    onSelect = { onSelectSession(session.id) },
                    onClose = { onCloseSession(session.id) },
                )
            }
        }
    }
}

@Composable
private fun SessionChildRow(
    label: String,
    state: TerminalSession.State,
    selected: Boolean,
    onSelect: () -> Unit,
    onClose: () -> Unit,
) {
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .clickable(onClick = onSelect)
            .padding(start = 20.dp, end = 8.dp, top = 4.dp, bottom = 4.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        StatusDot(kind = state.statusKind())
        Spacer(Modifier.width(4.dp))
        Column(modifier = Modifier.weight(1f).padding(horizontal = 8.dp)) {
            Text(
                label,
                style = MaterialTheme.typography.bodyMedium,
                fontWeight = FontWeight.SemiBold,
                color = if (selected) maineSelectionColor()
                else MaterialTheme.colorScheme.onSurface,
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
            onClick = onClose,
            modifier = Modifier.size(40.dp),
        ) {
            Icon(Icons.Filled.Close, contentDescription = "Close session")
        }
    }
}
