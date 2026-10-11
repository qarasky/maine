package app.maine.android.session

import app.maine.android.model.Host
import net.schmizz.sshj.SSHClient
import net.schmizz.sshj.connection.channel.direct.Session
import net.schmizz.sshj.transport.verification.PromiscuousVerifier
import java.io.IOException
import java.io.OutputStream
import java.util.concurrent.Executors
import java.util.concurrent.RejectedExecutionException
import java.util.concurrent.atomic.AtomicBoolean

/**
 * The live transport: an interactive shell on a remote host, opened with sshj
 * and presented to the terminal as a plain byte stream.
 *
 * **Every sshj call happens off the main thread.** Android throws
 * `NetworkOnMainThreadException` for socket work on the UI thread, and sshj
 * writes to the socket from whichever thread calls it — a window-change, a
 * keystroke or a disconnect issued from the UI thread aborts mid-packet and
 * either crashes the app or corrupts the SSH stream, which the server answers
 * with a connection reset. So connecting and reading run on one worker thread,
 * and `send`, `resize` and `close` are all handed to a single-threaded
 * executor. That executor also coalesces the burst of resizes a keyboard
 * animation produces.
 *
 * Host keys are accepted without checking — the equivalent of
 * `StrictHostKeyChecking=no`. macOS defers to the user's own `ssh`, which pins
 * keys in `known_hosts`; Android has no such file, so pinning needs a
 * vault-backed fingerprint on the host and is not implemented yet.
 *
 * Password authentication only: there is no `~/.ssh` on Android to read keys
 * from. Key import from the vault is a later pass.
 */
class SshjConnection : SshConnection {

    /** Serializes every socket touch that is not already on a background thread. */
    private val io = Executors.newSingleThreadExecutor { runnable ->
        Thread(runnable, "maine-ssh-io").apply { isDaemon = true }
    }

    /** Guards against tearing the session down twice. */
    private val torn = AtomicBoolean(false)

    @Volatile
    private var client: SSHClient? = null

    @Volatile
    private var shell: Session.Shell? = null

    @Volatile
    private var stdin: OutputStream? = null

    @Volatile
    private var closed = false

    /** Latest size the view reported; used for the initial PTY and later resizes. */
    @Volatile
    private var ptyColumns = DEFAULT_COLUMNS

    @Volatile
    private var ptyRows = DEFAULT_ROWS

    /** Size the server has actually been told about, so resizes are not repeated. */
    @Volatile
    private var sentColumns = 0

    @Volatile
    private var sentRows = 0

    override fun start(host: Host, password: String?, listener: SshConnection.Listener) {
        Thread({ run(host, password, listener) }, "maine-ssh").apply {
            isDaemon = true
            start()
        }
    }

    /**
     * Safe to call from any thread: the emulator's writer thread drives normal
     * typing, but the on-screen keys row calls in from the UI thread, and sshj
     * writes to the socket from whichever thread invokes it — which Android
     * refuses with `NetworkOnMainThreadException`. The bytes are copied here
     * because the caller reuses its buffer and this outlives the call, and the
     * switch to the I/O thread also keeps every write serialized.
     */
    override fun send(bytes: ByteArray, offset: Int, length: Int) {
        if (length <= 0) return
        val payload = bytes.copyOfRange(offset, offset + length)
        submit {
            val target = stdin ?: return@submit
            try {
                target.write(payload)
                target.flush()
            } catch (e: IOException) {
                // Channel is gone; the reader thread reports the failure.
            }
        }
    }

    override fun resize(columns: Int, rows: Int) {
        ptyColumns = columns
        ptyRows = rows
        // Many of these arrive in a burst while the keyboard animates; each task
        // sends the newest size and skips if the server already has it, so the
        // burst collapses to at most one window-change.
        submit {
            val channel = shell ?: return@submit
            if (closed) return@submit
            val c = ptyColumns
            val r = ptyRows
            if (c == sentColumns && r == sentRows) return@submit
            try {
                channel.changeWindowDimensions(c, r, 0, 0)
                sentColumns = c
                sentRows = r
            } catch (e: Exception) {
                // A failed resize is not fatal: the shell keeps its old size.
            }
        }
    }

    override fun close() {
        closed = true
        submit {
            teardown()
            io.shutdown()
        }
    }

    private fun submit(task: () -> Unit) {
        try {
            io.execute(task)
        } catch (e: RejectedExecutionException) {
            // Already shut down; nothing left to do.
        }
    }

    /** Closes the channel and the client exactly once. Touches the network. */
    private fun teardown() {
        if (!torn.compareAndSet(false, true)) return
        val channel = shell
        shell = null
        stdin = null
        runCatching { channel?.close() }
        val ssh = client
        client = null
        runCatching { ssh?.disconnect() }
    }

    private fun run(host: Host, password: String?, listener: SshConnection.Listener) {
        var ssh: SSHClient? = null
        try {
            val user = host.username.trim()
            if (user.isEmpty()) {
                throw IllegalStateException("This host has no username, so it cannot sign in.")
            }
            if (password.isNullOrEmpty()) {
                throw IllegalStateException("No password is saved for this host.")
            }

            ssh = SSHClient()
            client = ssh
            ssh.addHostKeyVerifier(PromiscuousVerifier())
            ssh.connectTimeout = CONNECT_TIMEOUT_MS
            ssh.connect(host.hostname, host.port)
            ssh.authPassword(user, password)
            if (closed) return

            val session = ssh.startSession()
            session.allocatePTY(TERM, ptyColumns, ptyRows, 0, 0, emptyMap())
            val channel = session.startShell()
            stdin = channel.outputStream
            shell = channel
            sentColumns = ptyColumns
            sentRows = ptyRows
            if (closed) return

            channel.inputStream.use { input ->
                val buffer = ByteArray(READ_BUFFER_BYTES)
                while (true) {
                    val read = input.read(buffer)
                    if (read == -1) break
                    if (closed) break
                    listener.onOutput(buffer, read)
                }
            }
            if (!closed) listener.onExit(0)
        } catch (e: Exception) {
            if (!closed) listener.onError(describe(e))
        } finally {
            // Already on a worker thread, so this may touch the socket directly.
            teardown()
        }
    }

    /** A message for the error banner. Never includes the password. */
    private fun describe(e: Exception): String {
        val detail = e.message?.trim().orEmpty()
        return detail.ifEmpty { e.javaClass.simpleName }
    }

    private companion object {
        const val TERM = "xterm-256color"
        const val DEFAULT_COLUMNS = 80
        const val DEFAULT_ROWS = 24
        const val CONNECT_TIMEOUT_MS = 15_000
        const val READ_BUFFER_BYTES = 8192
    }
}
