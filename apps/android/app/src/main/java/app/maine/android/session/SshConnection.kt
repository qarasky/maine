package app.maine.android.session

import app.maine.android.model.Host

/**
 * The transport behind one terminal session.
 *
 * Byte-oriented on purpose: a terminal emulator consumes raw bytes, and so
 * does an SSH channel. A real sshj session's remote output stream becomes
 * [Listener.onOutput] and its stdin stream is fed by [send], so the emulator
 * and the session state machine never need to know which transport is behind
 * them. [FakeSshConnection] backs previews, screenshots and the emulator
 * before real credentials exist.
 */
interface SshConnection {
    /** Listener for transport events. All callbacks arrive off the UI thread. */
    interface Listener {
        /**
         * Raw remote output. Multi-byte UTF-8 sequences may be split across
         * calls; the terminal emulator consumes the byte stream as-is.
         */
        fun onOutput(bytes: ByteArray, length: Int)

        fun onExit(code: Int)
        fun onError(message: String)
    }

    fun start(host: Host, password: String?, listener: Listener)

    /** Raw user input destined for the remote shell. */
    fun send(bytes: ByteArray, offset: Int, length: Int)

    /**
     * Tells the remote side how big the terminal now is. Called from the UI
     * thread whenever the view is resized, possibly before [start] completes.
     */
    fun resize(columns: Int, rows: Int)

    fun close()
}

/**
 * A tiny scripted shell that makes the terminal emulator demonstrable without
 * a server: it greets with an ANSI-colored banner, echoes typed characters,
 * handles backspace and answers Enter with a prompt. It is not a shell — it
 * exists so color, cursor, line editing and scrollback can be exercised on a
 * device before the sshj transport lands.
 */
class FakeSshConnection : SshConnection {
    private var listener: SshConnection.Listener? = null
    private var thread: Thread? = null
    private val line = StringBuilder()

    @Volatile
    private var closed = false

    override fun start(host: Host, password: String?, listener: SshConnection.Listener) {
        this.listener = listener
        thread = Thread({
            try {
                Thread.sleep(300)
                if (closed) return@Thread
                emit(
                    "\r\n" +
                        "\u001B[1;33mMaine\u001B[0m preview shell. No server is contacted.\r\n" +
                        "\u001B[38;5;245mConnected to ${host.subtitle}\u001B[0m\r\n\r\n" +
                        prompt(),
                )
            } catch (e: InterruptedException) {
                // Closed before the greeting flushed.
            }
        }, "fake-ssh").also { it.isDaemon = true; it.start() }
    }

    override fun send(bytes: ByteArray, offset: Int, length: Int) {
        if (closed) return
        for (i in offset until offset + length) {
            when (val byte = bytes[i].toInt() and 0xFF) {
                '\r'.code, '\n'.code -> {
                    val command = line.toString().trim()
                    line.clear()
                    emit("\r\n" + respond(command) + "\r\n" + prompt())
                }
                // Backspace and delete both erase locally, like a real line discipline.
                8, 127 -> if (line.isNotEmpty()) {
                    line.deleteCharAt(line.length - 1)
                    emit("\b \b")
                }
                in 32..126 -> {
                    line.append(byte.toChar())
                    emit(String(byteArrayOf(bytes[i])))
                }
                // Control characters other than Enter are ignored.
                else -> Unit
            }
        }
    }

    override fun close() {
        closed = true
        thread?.interrupt()
        thread = null
        listener = null
    }

    override fun resize(columns: Int, rows: Int) = Unit

    private fun respond(command: String): String = when (command) {
        "" -> ""
        "ls" -> "\u001B[38;5;220mhosts.json\u001B[0m  " +
            "\u001B[38;5;220mpasswords.maine\u001B[0m  README.md"
        "whoami" -> "tester"
        else -> "\u001B[38;5;203msh: $command: command not found\u001B[0m"
    }

    private fun prompt(): String =
        "\u001B[38;5;220mtester@maine\u001B[0m:\u001B[38;5;245m~\u001B[0m$ "

    private fun emit(text: String) {
        val target = listener ?: return
        val bytes = text.toByteArray(Charsets.UTF_8)
        target.onOutput(bytes, bytes.size)
    }
}
