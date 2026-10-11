package app.maine.android.session

import android.content.Context
import app.maine.android.model.Host
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import java.io.ByteArrayOutputStream
import java.util.UUID

/**
 * One interactive SSH session for a host.
 *
 * Mirrors the macOS `TerminalSession` state machine: six states, history
 * kept as plain-text scrollback capped at 1 MiB, and restored history fed
 * only to the display — never replayed to the transport.
 *
 * Display is a real terminal emulator ([SshTerminal], vendored from Termux):
 * it is created lazily when a view attaches, so sessions restored at launch
 * (and JVM unit tests, which have no Looper) never construct one. Until a
 * view attaches, remote bytes are buffered so nothing shown before layout is
 * lost.
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

        /** Cap on output buffered before a terminal view attaches. */
        private const val MAX_PENDING_BYTES = 256 * 1024

        /**
         * Cap on restored scrollback replayed into the emulator. The emulator
         * only keeps a few thousand rows anyway, so this never loses anything
         * a user could scroll back to.
         */
        private const val MAX_SCROLLBACK_CHARS = 256 * 1024
    }

    private val _state = MutableStateFlow<State>(State.Idle)
    val state: StateFlow<State> = _state.asStateFlow()

    /**
     * The terminal emulator, created on first use. Must be created on the main
     * thread (the emulator binds a Handler to it), which is why this is lazy
     * rather than a constructor field.
     */
    @Volatile
    private var emulatorInstance: SshTerminal? = null

    /**
     * Remote bytes that arrived before a view attached, so nothing is lost
     * while a restored session waits for its screen to compose. Bounded: only
     * the most recent [MAX_PENDING_BYTES] survive.
     */
    private val pendingOutput = ByteArrayOutputStream()
    private val emulatorLock = Any()

    /**
     * Saved scrollback waiting to be replayed. It is handed to the emulator
     * once, when the emulator is created, rather than pushed through the byte
     * stream: replayed output must not be answered (see
     * [SshTerminal.initialScrollback]). Bounded like the history it comes from.
     */
    private val pendingScrollback = StringBuilder()

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
            override fun onOutput(bytes: ByteArray, length: Int) {
                routeToEmulator(bytes, length)
                appendHistory(String(bytes, 0, length, Charsets.UTF_8))
                if (_state.value is State.Connecting) {
                    _state.value = State.Connected
                }
            }

            override fun onExit(code: Int) {
                _state.value = State.Closed(code)
                // Stop the emulator too, so its read/write threads end with the
                // transport instead of lingering on a dead connection.
                emulatorInstance?.finish()
            }

            override fun onError(message: String) {
                _state.value = State.Failed(message)
                emulatorInstance?.finish()
            }
        })
    }

    /**
     * The terminal emulator for this session, creating it on first call.
     * Main thread only. Buffered output is replayed before any live output.
     */
    fun emulator(context: Context): SshTerminal = synchronized(emulatorLock) {
        emulatorInstance?.let { return it }
        val created = SshTerminal(
            appContext = context.applicationContext,
            send = { bytes, offset, length -> sendBytes(bytes, offset, length) },
            onResize = { columns, rows -> connection?.resize(columns, rows) },
            scrollback = {
                synchronized(emulatorLock) {
                    if (pendingScrollback.isEmpty()) {
                        null
                    } else {
                        val bytes = pendingScrollback.toString().toByteArray(Charsets.UTF_8)
                        pendingScrollback.setLength(0)
                        bytes
                    }
                }
            },
        )
        emulatorInstance = created
        if (pendingOutput.size() > 0) {
            val buffered = pendingOutput.toByteArray()
            pendingOutput.reset()
            created.append(buffered, buffered.size)
        }
        created
    }

    private fun routeToEmulator(bytes: ByteArray, length: Int) {
        synchronized(emulatorLock) {
            val terminal = emulatorInstance
            if (terminal != null) {
                terminal.append(bytes, length)
                return
            }
            if (length <= 0) return
            if (pendingOutput.size() + length > MAX_PENDING_BYTES) {
                // Keep the tail: a view attaches within a frame or two, so this
                // only ever trims the far end of a long restored transcript.
                val kept = pendingOutput.toByteArray().takeLast(MAX_PENDING_BYTES / 2).toByteArray()
                pendingOutput.reset()
                pendingOutput.write(kept)
            }
            pendingOutput.write(bytes, 0, length)
        }
    }

    fun send(input: String) {
        if (!_state.value.isLive) return
        val bytes = input.toByteArray(Charsets.UTF_8)
        connection?.send(bytes, 0, bytes.size)
        // Local echo happens when the transport answers; the fake echoes.
    }

    /** Raw terminal input, straight from the emulator's key handling. */
    internal fun sendBytes(bytes: ByteArray, offset: Int, length: Int) {
        if (!_state.value.isLive) return
        connection?.send(bytes, offset, length)
    }

    /**
     * Sends a special key from the on-screen keys row. No-op until the terminal
     * emulator exists, which is also the point at which the row can appear.
     */
    fun sendSpecialKey(keyCode: Int, keyMod: Int) {
        emulatorInstance?.sendSpecialKey(keyCode, keyMod)
    }

    fun disconnect() {
        intentionalDisconnect = true
        connection?.close()
        connection = null
        emulatorInstance?.finish()
        if (_state.value.isLive) _state.value = State.Disconnected(null)
    }

    fun wasIntentionalDisconnect(): Boolean = intentionalDisconnect

    /** Feeds restored history to the display only. Never touches the transport. */
    fun restoreHistory(saved: String) {
        appendHistory(saved)
        synchronized(emulatorLock) {
            pendingScrollback.append(saved)
            if (pendingScrollback.length > MAX_SCROLLBACK_CHARS) {
                pendingScrollback.delete(0, pendingScrollback.length - MAX_SCROLLBACK_CHARS)
            }
        }
    }

    fun historySnapshot(): String {
        synchronized(historyLock) {
            val text = history.toString()
            return if (text.length <= MAX_HISTORY_CHARS) text
            else text.substring(text.length - MAX_HISTORY_CHARS)
        }
    }

    private fun appendHistory(text: String) {
        // Append-only: the emulator owns what is on screen, so no snapshot is
        // built per output chunk (that used to copy up to 1 MiB each time).
        synchronized(historyLock) {
            history.append(text)
            if (history.length > MAX_HISTORY_CHARS + 4096) {
                history.delete(0, history.length - MAX_HISTORY_CHARS)
            }
        }
    }
}
