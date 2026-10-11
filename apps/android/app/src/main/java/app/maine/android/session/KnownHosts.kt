package app.maine.android.session

import java.io.File

/**
 * Trust-on-first-use store for SSH host keys — the equivalent of
 * `~/.ssh/known_hosts`.
 *
 * It lives beside the vault rather than inside it on purpose. Trust is this
 * device's own view of a server, which is why `ssh` keeps it in a separate
 * file, and keeping it out of the vault leaves `passwords.maine`
 * byte-compatible with the other clients.
 *
 * Entries are keyed by host UUID, so a pin follows its host record and is
 * dropped when that host is deleted — which is also the documented way to
 * re-trust a server that was legitimately rebuilt.
 *
 * Only public key fingerprints are stored, so the file is not sensitive and
 * needs no encryption; it lives in app-private storage regardless.
 */
class KnownHosts(private val file: File) {

    data class Entry(val keyType: String, val fingerprint: String)

    private val entries = mutableMapOf<String, Entry>()
    private var loaded = false

    /** The pinned key for a host, or null if it has never been seen. */
    @Synchronized
    fun entry(hostId: String): Entry? {
        ensureLoaded()
        return entries[hostId]
    }

    /** Records a host's key the first time it is seen. */
    @Synchronized
    fun pin(hostId: String, entry: Entry) {
        ensureLoaded()
        entries[hostId] = entry
        persist()
    }

    /** Forgets a host's key, so the next connection pins again. */
    @Synchronized
    fun forget(hostId: String) {
        ensureLoaded()
        if (entries.remove(hostId) != null) persist()
    }

    companion object {
        /** Beside the vault, in app-private storage. */
        const val FILE_NAME = "known_hosts"
    }

    private fun ensureLoaded() {
        if (loaded) return
        loaded = true
        if (!file.exists()) return
        try {
            file.forEachLine { line ->
                val parts = line.trim().split(' ')
                if (parts.size >= 3) {
                    entries[parts[0]] = Entry(parts[1], parts[2])
                }
            }
        } catch (e: Exception) {
            // An unreadable store is treated as empty, so every host pins
            // again. Failing closed here would lock the user out of every host
            // to defend against an attacker who can already write our files.
        }
    }

    /** Writes the whole file via a rename, so a crash cannot truncate it. */
    private fun persist() {
        try {
            val text = entries.entries.joinToString("\n") { (id, entry) ->
                "$id ${entry.keyType} ${entry.fingerprint}"
            }
            val temporary = File(file.parentFile, "${file.name}.tmp")
            temporary.writeText(text)
            if (!temporary.renameTo(file)) {
                file.writeText(text)
                temporary.delete()
            }
        } catch (e: Exception) {
            // Losing a pin only costs a re-pin on the next connection.
        }
    }
}
