package app.maine.android.session

import android.content.Context
import android.content.ClipData
import android.content.ClipboardManager
import com.termux.terminal.KeyHandler
import com.termux.terminal.TerminalSessionClient
import java.io.InputStream
import java.io.OutputStream
import java.util.concurrent.LinkedBlockingDeque
import java.util.concurrent.TimeUnit
import com.termux.terminal.TerminalSession as TermuxSession

/**
 * Bridges one Maine SSH session to the vendored terminal emulator.
 *
 * The emulator is fed exactly the bytes that arrive from the transport, so it
 * owns cursor, colors, alternate screen, scrollback and line editing. User
 * input flows back the other way: [TermuxSession]'s writer thread drains its
 * input queue into [getOutputStream], whose `write` hands the bytes to the SSH
 * transport. Nothing in between interprets or rewrites them.
 *
 * Must be constructed on a thread with a Looper — in practice the main thread,
 * because [TermuxSession] binds a `Handler` to the constructing thread and
 * delivers all emulator callbacks there.
 */
class SshTerminal(
    private val appContext: Context,
    private val send: (bytes: ByteArray, offset: Int, length: Int) -> Unit,
    private val onResize: (columns: Int, rows: Int) -> Unit,
    private val scrollback: () -> ByteArray?,
) : TermuxSession.Transport, TerminalSessionClient {

    /**
     * Remote output queue. Unbounded until the reader thread starts draining it
     * (the emulator is only pumped once the view reports its size), so nothing
     * that arrives during layout is lost.
     */
    private val remoteOutput = QueueInputStream()

    private val remoteInput = object : OutputStream() {
        override fun write(b: Int) = write(byteArrayOf(b.toByte()), 0, 1)

        override fun write(b: ByteArray, off: Int, len: Int) {
            send(b, off, len)
        }
    }

    /** The emulator session. Create on the main thread: it binds a Handler. */
    val session: TermuxSession = TermuxSession(this, TRANSCRIPT_ROWS, this)

    /**
     * Screen-changed hook, owned by the attached [com.termux.view.TerminalView].
     * Set on attach and cleared on release so a destroyed view cannot be leaked.
     */
    var onScreenUpdated: (() -> Unit)? = null

    /** Feeds remote output bytes to the emulator. Callable from any thread. */
    fun append(bytes: ByteArray, length: Int) {
        remoteOutput.feed(bytes, length)
    }

    /** Ends the emulator session, which shuts this transport down. */
    fun finish() {
        session.finishIfRunning()
    }

    /**
     * Sends a special key (Escape, Tab, an arrow) as a VT sequence, honouring
     * the emulator's application-cursor and keypad modes — so arrows behave
     * inside `vim` and `less` exactly as they would on a real terminal rather
     * than always sending the base sequence.
     */
    fun sendSpecialKey(keyCode: Int, keyMod: Int) {
        val emulator = session.emulator ?: return
        val sequence = KeyHandler.getCode(
            keyCode,
            keyMod,
            emulator.isCursorKeysApplicationMode(),
            emulator.isKeypadApplicationMode(),
        ) ?: return
        val bytes = sequence.toByteArray(Charsets.UTF_8)
        send(bytes, 0, bytes.size)
    }

    // MARK: - TermuxSession.Transport

    override fun getInputStream(): InputStream = remoteOutput

    override fun getOutputStream(): OutputStream = remoteInput

    override fun resize(columns: Int, rows: Int) = onResize(columns, rows)

    override fun initialScrollback(): ByteArray? = scrollback()

    override fun close() {
        remoteOutput.shutdown()
    }

    // MARK: - TerminalSessionClient
    //
    // Emulator callbacks all arrive on the main thread. Logging is deliberately
    // silent: the diagnostics invariant forbids writing terminal contents (and
    // these callbacks carry them) anywhere, including logcat.

    override fun onTextChanged(changedSession: TermuxSession) {
        onScreenUpdated?.invoke()
    }

    override fun onTitleChanged(changedSession: TermuxSession) = Unit

    override fun onSessionFinished(finishedSession: TermuxSession) {
        onScreenUpdated?.invoke()
    }

    override fun onCopyTextToClipboard(session: TermuxSession, text: String) {
        val clipboard = appContext.getSystemService(ClipboardManager::class.java) ?: return
        clipboard.setPrimaryClip(ClipData.newPlainText("Maine", text))
    }

    override fun onPasteTextFromClipboard(session: TermuxSession?) {
        val clipboard = appContext.getSystemService(ClipboardManager::class.java) ?: return
        val text = clipboard.primaryClip?.getItemAt(0)?.coerceToText(appContext)?.toString()
        if (!text.isNullOrEmpty()) session?.write(text)
    }

    override fun onBell(session: TermuxSession) = Unit

    override fun onColorsChanged(session: TermuxSession) = Unit

    override fun onTerminalCursorStateChange(state: Boolean) = Unit

    override fun setTerminalShellPid(session: TermuxSession, pid: Int) = Unit

    override fun getTerminalCursorStyle(): Int? = null

    override fun logError(tag: String, message: String) = Unit
    override fun logWarn(tag: String, message: String) = Unit
    override fun logInfo(tag: String, message: String) = Unit
    override fun logDebug(tag: String, message: String) = Unit
    override fun logVerbose(tag: String, message: String) = Unit
    override fun logStackTraceWithMessage(tag: String, message: String, e: Exception) = Unit
    override fun logStackTrace(tag: String, e: Exception) = Unit

    companion object {
        /**
         * Scrollback rows kept by the emulator. The terminal's own transcript,
         * above the persisted plain-text snapshot, which is capped separately.
         */
        const val TRANSCRIPT_ROWS = 4000
    }
}

/**
 * A blocking [InputStream] over queued byte chunks.
 *
 * The emulator's reader thread blocks in `read` until remote output arrives or
 * the stream is shut down, which is how it learns a session ended. Feeds come
 * from the transport thread, reads from the emulator thread, so chunk handoff
 * avoids copying except when a caller's buffer spans two chunks.
 */
private class QueueInputStream : InputStream() {
    private val chunks = LinkedBlockingDeque<ByteArray>()

    /** Only ever touched by the reading thread. */
    private var current: ByteArray? = null
    private var offset = 0

    @Volatile
    private var closed = false

    fun feed(bytes: ByteArray, length: Int) {
        if (closed || length <= 0) return
        chunks.addLast(bytes.copyOf(length))
    }

    fun shutdown() {
        closed = true
    }

    override fun read(): Int {
        val single = ByteArray(1)
        return if (read(single, 0, 1) <= 0) -1 else single[0].toInt() and 0xFF
    }

    override fun read(b: ByteArray, off: Int, len: Int): Int {
        if (len == 0) return 0
        while (true) {
            val chunk = current
            if (chunk != null) {
                val n = minOf(chunk.size - offset, len)
                System.arraycopy(chunk, offset, b, off, n)
                offset += n
                if (offset >= chunk.size) {
                    current = null
                    offset = 0
                }
                return n
            }
            if (closed) return -1
            val next = try {
                // Timed so shutdown is noticed without a wakeup per idle block.
                chunks.pollFirst(200, TimeUnit.MILLISECONDS)
            } catch (e: InterruptedException) {
                return -1
            }
            if (closed) return -1
            if (next != null) current = next
        }
    }

    override fun close() = shutdown()
}
