package app.maine.android.ssh

import app.maine.android.model.Host

/**
 * The SSH endpoint Maine connects to for a host.
 *
 * Android has no system OpenSSH client, so this type carries the connection
 * parameters (used by the sshj transport) plus the equivalent OpenSSH
 * argument list for display. The display rules match the macOS client
 * exactly: as little as possible is pinned down so server-side config keeps
 * working.
 *
 * * port 22 means "default": no `-p` flag, the server side decides;
 * * empty username means "not specified": no `-l` flag;
 * * the hostname is always the final argument, and is what sshj dials.
 *
 * No password or private key ever passes through here.
 */
data class SshTarget(
    /** Display name of the client, for the connection header tooltip. */
    val executable: String = DISPLAY_EXECUTABLE,
    val arguments: List<String> = emptyList(),
    /** What to actually dial: resolved separately from display args. */
    val hostname: String = "",
    val port: Int = Host.DEFAULT_PORT,
    val username: String = "",
) {
    companion object {
        const val DISPLAY_EXECUTABLE = "ssh"

        fun make(host: Host, config: SshConfigFile = SshConfigFile()): SshTarget {
            val arguments = mutableListOf<String>()
            if (host.port != Host.DEFAULT_PORT) {
                arguments += listOf("-p", host.port.toString())
            }
            if (host.username.isNotEmpty()) {
                arguments += listOf("-l", host.username)
            }
            arguments += host.hostname

            val entry = config.entryFor(host.hostname)
            val resolvedPort = if (host.port != Host.DEFAULT_PORT) {
                host.port
            } else {
                entry?.port ?: Host.DEFAULT_PORT
            }
            return SshTarget(
                arguments = arguments,
                hostname = entry?.hostname ?: host.hostname,
                port = resolvedPort,
                username = host.username.ifEmpty { entry?.user ?: "" },
            )
        }
    }

    /** The equivalent command line, for display and copying to a terminal. */
    val commandLine: String get() = (listOf(executable) + arguments).joinToString(" ")

    /** Best-effort human explanation of what will be connected to. */
    fun resolved(using: SshConfigFile): Resolution {
        val address = arguments.lastOrNull() ?: ""
        val entry = using.entryFor(address)
        val user = firstValueOf("-l") ?: entry?.user ?: localUsername()
        val resolvedPort = firstValueOf("-p")?.toIntOrNull()
            ?: entry?.port
            ?: Host.DEFAULT_PORT
        return Resolution(
            hostname = entry?.hostname ?: address,
            username = user,
            port = resolvedPort,
            configPattern = entry?.pattern,
        )
    }

    private fun firstValueOf(flag: String): String? {
        val index = arguments.indexOf(flag)
        return if (index >= 0 && index + 1 < arguments.size) arguments[index + 1] else null
    }

    private fun localUsername(): String =
        System.getProperty("user.name")?.takeIf { it.isNotBlank() } ?: "user"

    data class Resolution(
        val hostname: String,
        val username: String,
        val port: Int,
        /** The config stanza that contributed these values, when there is one. */
        val configPattern: String? = null,
    ) {
        val target: String get() = "$username@$hostname:$port"
    }
}
