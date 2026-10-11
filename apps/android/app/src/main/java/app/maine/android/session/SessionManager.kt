package app.maine.android.session

import app.maine.android.model.Host
import app.maine.android.model.SavedSession
import app.maine.android.store.HostStore
import app.maine.android.store.HostStoreError
import app.maine.android.vault.VaultStore
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch

/**
 * Tracks the open terminal sessions and which one is shown.
 *
 * Sessions have independent identities even when they connect to the same
 * host: everything is keyed by session ID, never host ID. Mirrors the macOS
 * `SessionManager`, including close-vs-quit: explicit close deletes the
 * encrypted session record first (a failed delete keeps the session
 * visible), while quit only stops processes and keeps records.
 */
class SessionManager(
    private val store: HostStore,
    private val vault: VaultStore,
    private val scope: CoroutineScope,
    private val makeSession: (Host) -> TerminalSession = { TerminalSession(host = it) },
) {
    private val _openSessions = MutableStateFlow<List<TerminalSession>>(emptyList())
    val openSessions: StateFlow<List<TerminalSession>> = _openSessions.asStateFlow()

    private val _savedSessions = MutableStateFlow<List<SavedSession>>(emptyList())
    val savedSessions: StateFlow<List<SavedSession>> = _savedSessions.asStateFlow()

    private val _selectedSessionId = MutableStateFlow<String?>(null)
    val selectedSessionId: StateFlow<String?> = _selectedSessionId.asStateFlow()

    val selectedSession: TerminalSession?
        get() = _openSessions.value.firstOrNull { it.id == _selectedSessionId.value }

    /** Host-level navigation for the host list. */
    var selectedHostId: String?
        get() = selectedSession?.host?.id
        set(value) {
            _selectedSessionId.value =
                if (value == null) null else sessionFor(value)?.id
        }

    private val sessionNumbers = mutableMapOf<String, Int>()
    private var autosaveJob: Job? = null

    // MARK: - Queries

    fun sessionFor(hostId: String): TerminalSession? {
        val selected = selectedSession
        if (selected != null && Host.idsEqual(selected.host.id, hostId)) return selected
        val mine = sessionsFor(hostId)
        return mine.firstOrNull { it.state.value.isLive } ?: mine.firstOrNull()
    }

    fun sessionsFor(hostId: String): List<TerminalSession> =
        _openSessions.value.filter { Host.idsEqual(it.host.id, hostId) }

    fun label(session: TerminalSession): String =
        session.customName ?: "Session ${sessionNumbers[session.id] ?: 1}"

    @Throws(HostStoreError::class)
    fun renameSession(id: String, name: String) {
        val trimmed = name.trim()
        if (trimmed.isEmpty()) throw HostStoreError("Enter a session name.")
        val session = _openSessions.value.firstOrNull { it.id == id } ?: return
        val savedId = session.savedSessionId
        if (savedId != null) {
            val saved = store.savedSessions().firstOrNull { Host.idsEqual(it.id, savedId) }
            if (saved != null) {
                store.saveSession(saved.copy(name = trimmed, usesDefaultName = false))
                _savedSessions.value = store.savedSessions()
            }
        }
        session.customName = trimmed
    }

    @Throws(Exception::class)
    fun checkpointSessions() {
        if (_openSessions.value.isEmpty()) return
        val stored = store.savedSessions()
        val changes = mutableListOf<SavedSession>()
        for (session in _openSessions.value) {
            val id = session.savedSessionId ?: session.id
            val history = session.historySnapshot()
            val name = label(session)
            val previous = stored.firstOrNull { Host.idsEqual(it.id, id) }
            if (previous != null && previous.name == name && previous.history == history) {
                continue
            }
            changes += SavedSession(
                id = id,
                hostID = session.host.id,
                name = name,
                history = history,
                usesDefaultName = session.customName == null,
                number = sessionNumbers[session.id],
            )
        }
        if (changes.isNotEmpty()) {
            store.saveSessions(changes)
            store.clearSaveFailure()
        }
        for (session in _openSessions.value) {
            if (session.savedSessionId == null) session.savedSessionId = session.id
        }
        _savedSessions.value = store.savedSessions()
    }

    private fun checkpointAutomatically() {
        try {
            checkpointSessions()
        } catch (e: VaultStore.VaultError.Locked) {
            // Resume on unlock; no plaintext fallback.
        } catch (e: Exception) {
            store.reportSaveFailure(e.message ?: "Maine could not save sessions.")
        }
    }

    private fun startAutosave() {
        if (autosaveJob != null) return
        autosaveJob = scope.launch {
            while (isActive) {
                delay(2000)
                checkpointAutomatically()
            }
        }
    }

    /** Quit ends processes but keeps their records, unlike explicit close. */
    @Throws(Exception::class)
    fun shutdown() {
        checkpointSessions()
        stopProcessesForQuit()
    }

    fun stopProcessesForQuit() {
        autosaveJob?.cancel()
        autosaveJob = null
        for (session in _openSessions.value) session.disconnect()
    }

    /** Restores snapshots and reconnects them, including after fresh launch. */
    @Throws(Exception::class)
    fun restoreSavedSessions(forHostId: String? = null) {
        _savedSessions.value = store.savedSessions()
        for (saved in _savedSessions.value) {
            if (forHostId != null && !Host.idsEqual(saved.hostID, forHostId)) continue
            if (_openSessions.value.any { it.savedSessionId?.let { id -> Host.idsEqual(id, saved.id) } == true }) {
                continue
            }
            val host = store.host(saved.hostID) ?: continue
            val session = makeSession(host)
            val generatedName = saved.usesDefaultName ?: DEFAULT_NAME_REGEX.matches(saved.name)
            session.customName = if (generatedName) null else saved.name
            session.savedSessionId = saved.id
            // Restored history stays as scrollback; the session reconnects
            // on its own so the user never has to press Connect after unlock.
            session.restoreHistory(separated(saved.history))
            sessionNumbers[session.id] = saved.number
                ?: (if (generatedName) saved.name.substringAfterLast(" ").toIntOrNull() else null)
                ?: (sessionsFor(host.id).size + 1)
            _openSessions.value += session
            session.connect(savedPassword(host))
        }
        if (_selectedSessionId.value == null) {
            _openSessions.value.firstOrNull()?.let { selectSession(it.id) }
        }
        startAutosave()
    }

    fun selectSession(sessionId: String) {
        if (_openSessions.value.any { it.id == sessionId }) {
            _selectedSessionId.value = sessionId
        }
    }

    fun select(hostId: String?) {
        selectedHostId = hostId?.takeIf { sessionFor(it) != null }
    }

    // MARK: - Opening and closing

    /** Connects to [host], or reveals the existing session when there is one. */
    fun open(host: Host) {
        val existing = sessionFor(host.id)
        if (existing != null) {
            if (!existing.state.value.isLive) {
                reconnect(host.id)
                return
            }
            _selectedSessionId.value = existing.id
            return
        }
        newSession(host)
    }

    /** Explicitly opens another independent connection, without replacing any. */
    fun newSession(host: Host) {
        val number = sessionsFor(host.id).mapNotNull { sessionNumbers[it.id] }.maxOrNull()
            ?.plus(1) ?: 1
        val session = makeSession(host)
        sessionNumbers[session.id] = number
        _openSessions.value += session
        _selectedSessionId.value = session.id
        session.connect(savedPassword(host))
        checkpointAutomatically()
        startAutosave()
    }

    /** Starts a brand new session: clean screen, custom name preserved. */
    fun reconnect(hostId: String) {
        val session = sessionFor(hostId)
        if (session != null) {
            reconnectSession(session.id)
            return
        }
        val host = store.host(hostId) ?: return
        open(host)
    }

    fun reconnectSession(sessionId: String) {
        val index = _openSessions.value.indexOfFirst { it.id == sessionId }
        if (index < 0) return
        val previous = _openSessions.value[index]
        val host = store.host(previous.host.id) ?: previous.host
        previous.disconnect()
        val replacement = makeSession(host)
        replacement.customName = previous.customName
        replacement.savedSessionId = previous.savedSessionId
        if (previous.savedSessionId != null) {
            replacement.restoreHistory(separated(previous.historySnapshot()))
        }
        sessionNumbers[replacement.id] = sessionNumbers.remove(previous.id) ?: 1
        // Publish one replacement, not an empty workspace followed by an open.
        _openSessions.value = _openSessions.value.toMutableList().also { it[index] = replacement }
        _selectedSessionId.value = replacement.id
        replacement.connect(savedPassword(host))
        checkpointAutomatically()
    }

    fun disconnect(hostId: String) {
        sessionFor(hostId)?.disconnect()
    }

    fun close(hostId: String) {
        val session = sessionFor(hostId) ?: return
        closeSession(session.id)
    }

    fun closeSession(sessionId: String) {
        val session = _openSessions.value.firstOrNull { it.id == sessionId } ?: return
        val savedId = session.savedSessionId
        if (savedId != null) {
            try {
                store.deleteSavedSession(savedId)
                _savedSessions.value = _savedSessions.value.filterNot {
                    Host.idsEqual(it.id, savedId)
                }
            } catch (e: Exception) {
                store.reportSaveFailure("Session not closed: ${e.message}")
                return
            }
            store.clearSaveFailure()
        }
        // Always stop the transport before dropping the session that owns it.
        session.disconnect()
        sessionNumbers.remove(session.id)
        val wasSelected = _selectedSessionId.value == sessionId
        _openSessions.value = _openSessions.value.filterNot { it.id == sessionId }
        if (wasSelected) {
            _selectedSessionId.value =
                sessionsFor(session.host.id).lastOrNull()?.id
                    ?: _openSessions.value.lastOrNull()?.id
        }
    }

    fun closeAll() {
        for (session in _openSessions.value.toList()) closeSession(session.id)
        if (_openSessions.value.isEmpty()) {
            autosaveJob?.cancel()
            autosaveJob = null
        }
    }

    /** Removes the host and stops whatever it had running. */
    fun delete(host: Host) {
        store.delete(host)
        if (store.host(host.id) == null) {
            _savedSessions.value = _savedSessions.value.filterNot {
                Host.idsEqual(it.hostID, host.id)
            }
            for (session in sessionsFor(host.id)) closeSession(session.id)
        }
    }

    private fun savedPassword(host: Host): String? =
        runCatching { vault.passwordFor(host.id) }.getOrNull()

    /**
     * Marks where the output of a (re)started connection begins, so its banner
     * does not run into the previous prompt on the same line — the scrollback
     * ends mid-line, right after the shell prompt.
     */
    private fun separated(history: String): String =
        if (history.isEmpty()) history else history + NEW_CONNECTION_SEPARATOR

    companion object {
        private val DEFAULT_NAME_REGEX = Regex("^Session [0-9]+$")

        /** CRLF so the separator always starts in the first column. */
        private const val NEW_CONNECTION_SEPARATOR = "\r\n— New connection —\r\n"
    }
}
