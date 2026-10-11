package app.maine.android.session

import app.maine.android.model.Host

/**
 * The transport behind one terminal session.
 *
 * The live sshj implementation lands with the connection pass; the UI and
 * [TerminalSession] only ever talk to this interface, so that pass changes
 * no call sites. [FakeSshConnection] backs previews, screenshots and the
 * emulator before real credentials exist.
 */
interface SshConnection {
    /** Listener for transport events. All callbacks arrive off the UI thread. */
    interface Listener {
        fun onOutput(text: String)
        fun onExit(code: Int)
        fun onError(message: String)
    }

    fun start(host: Host, password: String?, listener: Listener)
    fun send(input: String)
    fun close()
}

/** Echoes input back as output so the session UI is testable without a server. */
class FakeSshConnection : SshConnection {
    private var listener: SshConnection.Listener? = null
    private var thread: Thread? = null

    override fun start(host: Host, password: String?, listener: SshConnection.Listener) {
        this.listener = listener
        thread = Thread({
            try {
                Thread.sleep(300)
                listener.onOutput("Connected to ${host.subtitle} (preview connection).\r\n")
            } catch (e: InterruptedException) {
                // Closed before the greeting flushed.
            }
        }, "fake-ssh").also { it.isDaemon = true; it.start() }
    }

    override fun send(input: String) {
        listener?.onOutput(input)
    }

    override fun close() {
        thread?.interrupt()
        thread = null
        listener = null
    }
}
