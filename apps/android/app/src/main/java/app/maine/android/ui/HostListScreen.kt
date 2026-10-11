package app.maine.android.ui

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Add
import androidx.compose.material.icons.filled.Close
import androidx.compose.material.icons.filled.Delete
import androidx.compose.material.icons.filled.Edit
import androidx.compose.material.icons.filled.Lock
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
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
                .padding(horizontal = 12.dp, vertical = 8.dp),
            horizontalArrangement = Arrangement.SpaceBetween,
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Text("Maine", style = MaterialTheme.typography.titleLarge)
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
            modifier = Modifier
                .fillMaxWidth()
                .padding(horizontal = 12.dp),
        )

        loadFailure?.let {
            Text(
                it.message,
                color = MaterialTheme.colorScheme.error,
                modifier = Modifier.padding(12.dp),
            )
        }
        saveFailure?.let {
            Text(
                it,
                color = MaterialTheme.colorScheme.error,
                modifier = Modifier.padding(12.dp),
            )
        }

        LazyColumn(modifier = Modifier.fillMaxSize()) {
            items(visible, key = { it.id }) { host ->
                val mine = sessions.filter { Host.idsEqual(it.host.id, host.id) }
                HostRow(
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
                HorizontalDivider()
            }
        }
    }
}

@Composable
private fun HostRow(
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
    val live = states.any { it.isLive }

    Column(modifier = Modifier.fillMaxWidth()) {
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .clickable(onClick = onOpen)
                .padding(horizontal = 12.dp, vertical = 10.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            StatusDot(connected = live)
            Column(modifier = Modifier.weight(1f).padding(horizontal = 8.dp)) {
                Text(host.displayName, style = MaterialTheme.typography.bodyLarge)
                Text(host.subtitle, style = MaterialTheme.typography.bodySmall)
            }
            IconButton(onClick = onNewSession) {
                Icon(Icons.Filled.Add, contentDescription = "New session")
            }
            IconButton(onClick = onEdit) {
                Icon(Icons.Filled.Edit, contentDescription = "Edit host")
            }
            IconButton(onClick = onDelete) {
                Icon(Icons.Filled.Delete, contentDescription = "Delete host")
            }
        }
        sessions.forEachIndexed { index, session ->
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
            .padding(start = 36.dp, end = 4.dp, top = 2.dp, bottom = 2.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        StatusDot(connected = state.isLive)
        Column(modifier = Modifier.weight(1f).padding(horizontal = 8.dp)) {
            Text(
                label,
                style = MaterialTheme.typography.bodyMedium,
                color = if (selected) MaterialTheme.colorScheme.primary
                else MaterialTheme.colorScheme.onSurface,
            )
            Text(state.label, style = MaterialTheme.typography.bodySmall)
        }
        IconButton(onClick = onClose) {
            Icon(Icons.Filled.Close, contentDescription = "Close session")
        }
    }
}
