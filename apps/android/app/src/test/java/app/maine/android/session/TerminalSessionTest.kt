package app.maine.android.session

import app.maine.android.model.Host
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class TerminalSessionTest {

    @Test
    fun connectsAndEchoes() {
        val session = TerminalSession(host = Host(name = "W", hostname = "example.com"))
        session.connect()
        awaitTrue { session.state.value is TerminalSession.State.Connected }
        assertTrue(session.historySnapshot().contains("example.com"))
        session.send("hello\n")
        awaitTrue { session.historySnapshot().contains("hello") }
        assertTrue(session.state.value.isLive)
        session.disconnect()
        assertTrue(session.state.value is TerminalSession.State.Disconnected)
    }

    @Test
    fun historySnapshotCapsAtOneMiB() {
        val session = TerminalSession(host = Host(name = "W", hostname = "h"))
        session.restoreHistory("x".repeat(TerminalSession.MAX_HISTORY_CHARS + 100))
        assertEquals(TerminalSession.MAX_HISTORY_CHARS, session.historySnapshot().length)
    }

    @Test
    fun intentionalDisconnectShowsNoBanner() {
        val closed = TerminalSession.State.Closed(255)
        assertTrue(closed.showsErrorBanner(wasIntentionalDisconnect = false))
        assertEquals(false, closed.showsErrorBanner(wasIntentionalDisconnect = true))
        assertTrue(
            TerminalSession.State.Failed("boom").showsErrorBanner(wasIntentionalDisconnect = true),
        )
        assertEquals(false, TerminalSession.State.Connected.showsErrorBanner(false))
    }

    companion object {
        /** Visible for the session-manager tests in this package. */
        fun awaitTrue(timeoutMs: Long = 5000, condition: () -> Boolean) {
            val deadline = System.currentTimeMillis() + timeoutMs
            while (!condition()) {
                if (System.currentTimeMillis() > deadline) {
                    throw AssertionError("condition not met within ${timeoutMs}ms")
                }
                Thread.sleep(25)
            }
        }
    }
}

fun testScope() = CoroutineScope(SupervisorJob() + Dispatchers.Default)
