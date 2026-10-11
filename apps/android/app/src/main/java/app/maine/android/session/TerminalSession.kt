package app.maine.android.session

import app.maine.android.model.Host
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import java.util.UUID

/**
 * One interactive SSH session for a host.
 *
 * Mirrors the macOS `TerminalSession` state machine: six states, history
 * kept as plain-text scrollback capped at 1 MiB, and restored history fed
 * only to the display — never replayed to the transport.
 */
class TerminalSession(
    val id: String = UUID.randomUUID().toString().uppercase(),
    val host: Host,
    private val makeConnection: () -> SshConnection = { FakeSshConnection() },
) {
    sealed class State {
        data object Idle : State()
        data object Connecting : State()
        data object Connected : State()
        data class Closed(val exitCode: Int?) : State()
        data class Disconnected(val exitCode: Int?) : State()
        data class Failed(val message: String) : State()

        val isLive: Boolean get() = this is Connecting || this is Connected

        /** Short label for the status indicator. */
        val label: String
            get() = when (this) {
                is Idle -> "Not connected"
                is Connecting -> "Connecting…"
                is Connected -> "Connected"
                is Closed -> "Closed"
                is Disconnected -> "Disconnected"
                is Failed -> "Failed"
            }

        /** Whether to show an error banner. An intentional disconnect stays
         * quiet even when the transport exits non-zero for being killed,
         * exactly like the macOS client. */
        fun showsErrorBanner(wasIntentionalDisconnect: Boolean): Boolean = when (this) {
            is Failed -> true
            is Closed -> (exitCode ?: 0) != 0 && !wasIntentionalDisconnect
            is Disconnected -> (exitCode ?: 0) != 0 && !wasIntentionalDisconnect
            else -> false
        }
    }

    companion object {
        /** History snapshots are capped at 1 MiB, like the macOS client. */
        const val MAX_HISTORY_CHARS = 1024 * 1024
    }

    private val _state = MutableStateFlow<State>(State.Idle)
    val state: StateFlow<State> = _state.asStateFlow()

    private val _output = MutableStateFlow("")
    /** Plain-text scrollback shown by the UI. */
    val output: StateFlow<String> = _output.asStateFlow()

    var customName: String? = null
    var savedSessionId: String? = null

    private var connection: SshConnection? = null
    private var intentionalDisconnect = false
    private val history = StringBuilder()
    private val historyLock = Any()

    fun connect(password: String? = null) {
        if (_state.value.isLive) return
        intentionalDisconnect = false
        _state.value = State.Connecting
        val connection = makeConnection().also { this.connection = it }
        connection.start(host, password, object : SshConnection.Listener {
            // StateFlow is thread-safe; callbacks arrive off the UI thread.
            override fun onOutput(text: String) {
                appendHistory(text)
                if (_state.value is State.Connecting) {
                    _state.value = State.Connected
                }
            }

            override fun onExit(code: Int) {
                _state.value = State.Closed(code)
            }

            override fun onError(message: String) {
                _state.value = State.Failed(message)
            }
        })
    }

    fun send(input: String) {
        if (!_state.value.isLive) return
        connection?.send(input)
        // Local echo happens when the transport answers; the fake echoes.
    }

    fun disconnect() {
        intentionalDisconnect = true
        connection?.close()
        connection = null
        if (_state.value.isLive) _state.value = State.Disconnected(null)
    }

    fun wasIntentionalDisconnect(): Boolean = intentionalDisconnect

    /** Feeds restored history to the display only. Never touches the transport. */
    fun restoreHistory(saved: String) {
        appendHistory(saved)
    }

    fun historySnapshot(): String {
        synchronized(historyLock) {
            val text = history.toString()
            return if (text.length <= MAX_HISTORY_CHARS) text
            else text.substring(text.length - MAX_HISTORY_CHARS)
        }
    }

    private fun appendHistory(text: String) {
        val snapshot = synchronized(historyLock) {
            history.append(text)
            if (history.length > MAX_HISTORY_CHARS + 4096) {
                history.delete(0, history.length - MAX_HISTORY_CHARS)
            }
            historySnapshot()
        }
        _output.value = snapshot
    }
}
