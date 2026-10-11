package app.maine.android.session

import app.maine.android.model.Host
import app.maine.android.session.TerminalSessionTest.Companion.awaitTrue
import app.maine.android.store.HostStore
import app.maine.android.vault.VaultStore
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.rules.TemporaryFolder

class SessionManagerTest {

    @get:Rule
    val temp = TemporaryFolder()

    private val password = "correct horse battery staple"

    private fun setup(): Triple<VaultStore, HostStore, Host> {
        val vault = VaultStore(temp.root.resolve("passwords.maine"))
        vault.unlock(password)
        val store = HostStore(vault)
        val host = Host(name = "Web", hostname = "203.0.113.10")
        vault.saveHost(host, "pw")
        store.load()
        return Triple(vault, store, host)
    }

    @Test
    fun openRevealsInsteadOfDuplicating() {
        val (vault, store, host) = setup()
        val manager = SessionManager(store, vault, testScope())
        manager.open(host)
        val first = manager.sessionFor(host.id)!!
        awaitTrue { first.state.value is TerminalSession.State.Connected }
        manager.open(host)
        assertEquals(1, manager.openSessions.value.size)
        assertEquals(first.id, manager.selectedSessionId.value)
    }

    @Test
    fun newSessionCreatesIndependentConnections() {
        val (vault, store, host) = setup()
        val manager = SessionManager(store, vault, testScope())
        manager.newSession(host)
        manager.newSession(host)
        assertEquals(2, manager.sessionsFor(host.id).size)
        assertEquals("Session 1", manager.label(manager.sessionsFor(host.id)[0]))
        assertEquals("Session 2", manager.label(manager.sessionsFor(host.id)[1]))
    }

    @Test
    fun renamePersistsAndCloseDeletesTheRecord() {
        val (vault, store, host) = setup()
        val manager = SessionManager(store, vault, testScope())
        manager.newSession(host)
        val session = manager.openSessions.value.single()
        awaitTrue { session.state.value is TerminalSession.State.Connected }
        manager.checkpointSessions()
        manager.renameSession(session.id, "DB shell")
        assertEquals("DB shell", manager.label(session))
        assertEquals("DB shell", vault.savedSessions().single().name)
        manager.closeSession(session.id)
        assertTrue(manager.openSessions.value.isEmpty())
        assertTrue(vault.savedSessions().isEmpty())
    }

    @Test
    fun restoreReconnectsWithScrollback() {
        val (vault, store, host) = setup()
        val first = SessionManager(store, vault, testScope())
        first.newSession(host)
        val session = first.openSessions.value.single()
        awaitTrue { session.state.value is TerminalSession.State.Connected }
        first.checkpointSessions()
        first.stopProcessesForQuit()
        // Records survive quit.
        assertEquals(1, vault.savedSessions().size)

        val second = SessionManager(store, vault, testScope())
        second.restoreSavedSessions()
        assertEquals(1, second.openSessions.value.size)
        val restored = second.openSessions.value.single()
        awaitTrue { restored.state.value is TerminalSession.State.Connected }
        assertTrue(restored.historySnapshot().contains("203.0.113.10"))
    }

    @Test
    fun reconnectKeepsNameAndMarksNewConnection() {
        val (vault, store, host) = setup()
        val manager = SessionManager(store, vault, testScope())
        manager.newSession(host)
        val session = manager.openSessions.value.single()
        awaitTrue { session.state.value is TerminalSession.State.Connected }
        manager.checkpointSessions()
        manager.renameSession(session.id, "DB shell")
        manager.reconnectSession(session.id)
        val replacement = manager.openSessions.value.single()
        assertEquals("DB shell", manager.label(replacement))
        awaitTrue { replacement.state.value is TerminalSession.State.Connected }
        assertTrue(replacement.historySnapshot().contains("— New connection —"))
    }

    @Test
    fun closingSelectedChildPrefersSameHost() {
        val (vault, store, host) = setup()
        val manager = SessionManager(store, vault, testScope())
        manager.newSession(host)
        manager.newSession(host)
        val sessions = manager.sessionsFor(host.id)
        assertEquals(2, sessions.size)
        manager.selectSession(sessions[1].id)
        manager.closeSession(sessions[1].id)
        assertEquals(sessions[0].id, manager.selectedSessionId.value)
        assertNull(manager.sessionsFor(host.id).firstOrNull { it.id == sessions[1].id })
    }
}
