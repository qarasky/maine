package app.maine.android.ssh

import java.io.File
import java.nio.file.FileSystems
import java.nio.file.Files
import java.nio.file.Path
import kotlin.io.path.absolute
import kotlin.io.path.exists
import kotlin.io.path.isRegularFile
import kotlin.io.path.readText

/**
 * One `Host` block found in an OpenSSH client configuration file.
 *
 * Only the directives needed to build a host record are read (`HostName`,
 * `User`, `Port`). Everything else stays with the SSH transport, exactly
 * like the macOS client, which never writes to this file.
 */
data class SshConfigEntry(
    val pattern: String,
    val hostname: String? = null,
    val user: String? = null,
    val port: Int? = null,
) {
    companion object {
        /** True when the pattern names exactly one host (no wildcards). */
        fun isConnectable(pattern: String): Boolean {
            if (pattern.isEmpty() || pattern.any { it.isWhitespace() }) return false
            return '*' !in pattern && '?' !in pattern
        }
    }
}

/**
 * A best-effort reader for the OpenSSH client configuration.
 *
 * Exists so imported hosts can fill in port/user defaults and so the UI can
 * show which stanza a connection comes from. Android devices normally have
 * no `~/.ssh/config`; a missing or unreadable file is an empty config, never
 * an error. `Include` chains are followed up to 8 deep with cycle protection,
 * matching the macOS parser.
 */
class SshConfigFile(val entries: List<SshConfigEntry> = emptyList()) {

    fun entryFor(pattern: String): SshConfigEntry? =
        entries.firstOrNull { it.pattern.equals(pattern, ignoreCase = true) }

    companion object {
        private const val MAX_INCLUDE_DEPTH = 8

        fun defaultFile(): File =
            File(System.getProperty("user.home") ?: ".", ".ssh/config")

        fun load(file: File = defaultFile()): SshConfigFile {
            val entries = mutableListOf<SshConfigEntry>()
            parseFile(file.toPath(), entries, mutableSetOf(), 0)
            return SshConfigFile(entries)
        }

        private fun parseFile(
            path: Path,
            entries: MutableList<SshConfigEntry>,
            visited: MutableSet<String>,
            depth: Int,
        ) {
            if (depth >= MAX_INCLUDE_DEPTH) return
            val normalized = runCatching { path.toAbsolutePath().normalize().toString() }
                .getOrNull() ?: return
            if (!visited.add(normalized)) return
            if (!path.exists() || !path.isRegularFile()) return
            val contents = runCatching { path.readText() }.getOrNull() ?: return

            var active: List<String> = emptyList()
            for (rawLine in contents.split("\n")) {
                val tokens = tokenize(rawLine)
                if (tokens.isEmpty()) continue
                val keyword = tokens[0].lowercase()
                val values = tokens.drop(1)
                when (keyword) {
                    "host" -> {
                        active = emptyList()
                        val patterns = mutableListOf<String>()
                        for (pattern in values) {
                            if (pattern.isEmpty()) continue
                            if (entries.none { it.pattern == pattern }) {
                                entries += SshConfigEntry(pattern)
                            }
                            patterns += pattern
                        }
                        active = patterns
                    }
                    "hostname" -> {
                        val value = values.firstOrNull() ?: continue
                        activeIndices(entries, active)
                            .filter { entries[it].hostname == null }
                            .forEach { entries[it] = entries[it].copy(hostname = value) }
                    }
                    "user" -> {
                        val value = values.firstOrNull() ?: continue
                        activeIndices(entries, active)
                            .filter { entries[it].user == null }
                            .forEach { entries[it] = entries[it].copy(user = value) }
                    }
                    "port" -> {
                        val port = values.firstOrNull()?.toIntOrNull() ?: continue
                        activeIndices(entries, active)
                            .filter { entries[it].port == null }
                            .forEach { entries[it] = entries[it].copy(port = port) }
                    }
                    "include" -> {
                        val folder = path.absolute().parent
                        for (argument in values) {
                            for (included in expandInclude(argument, folder)) {
                                parseFile(
                                    java.nio.file.Paths.get(included),
                                    entries,
                                    visited,
                                    depth + 1,
                                )
                            }
                        }
                    }
                    else -> {
                        // Not needed to build a host record.
                    }
                }
            }
        }

        private fun activeIndices(
            entries: List<SshConfigEntry>,
            patterns: List<String>,
        ): List<Int> = entries.indices.filter { patterns.contains(entries[it].pattern) }

        /** Splits one line into tokens, dropping comments and `Keyword=value`. */
        internal fun tokenize(line: String): List<String> {
            val tokens = mutableListOf<String>()
            val current = StringBuilder()
            for (c in stripComment(line)) {
                if (c == ' ' || c == '\t' || c == '=') {
                    if (current.isNotEmpty()) {
                        tokens += unquote(current.toString())
                        current.clear()
                    }
                } else {
                    current.append(c)
                }
            }
            if (current.isNotEmpty()) tokens += unquote(current.toString())
            return tokens
        }

        /** Drops a trailing `#` comment. `#` opens a comment only at the
         * start of a line or after whitespace, so values containing one
         * survive. */
        internal fun stripComment(line: String): String {
            val hash = line.indexOf('#')
            if (hash < 0) return line
            if (hash > 0 && !line[hash - 1].isWhitespace()) return line
            return line.substring(0, hash)
        }

        internal fun unquote(token: String): String {
            if (token.length >= 2 &&
                ((token.startsWith("\"") && token.endsWith("\"")) ||
                    (token.startsWith("'") && token.endsWith("'")))
            ) {
                return token.substring(1, token.length - 1)
            }
            return token
        }

        /** Expands `~`, braces and wildcards in an `Include` argument.
         * Relative arguments resolve against the containing file's folder. */
        internal fun expandInclude(argument: String, folder: Path): List<String> {
            val expanded = if (argument.startsWith("~")) {
                (System.getProperty("user.home") ?: "") + argument.substring(1)
            } else {
                argument
            }
            val full = if (expanded.startsWith("/")) expanded
            else folder.resolve(expanded).toString()
            return try {
                val matcher = FileSystems.getDefault().getPathMatcher("glob:$full")
                val base = full.substringBefore('*').substringBefore('?')
                    .substringBefore('{').substringBeforeLast('/', "")
                val roots = if (base.isEmpty()) listOf(folder) else listOf(
                    runCatching {
                        val p = java.nio.file.Paths.get(base)
                        if (p.isAbsolute) p else folder.fileSystem.getPath(base)
                    }.getOrNull() ?: folder,
                )
                val matches = mutableListOf<Path>()
                for (root in roots) {
                    if (!root.exists()) continue
                    Files.walk(root).use { stream ->
                        stream.filter { it.isRegularFile() && matcher.matches(it.absolute()) }
                            .forEach { matches.add(it) }
                    }
                }
                matches.map { it.toString() }.sorted()
            } catch (e: Exception) {
                val direct = java.nio.file.Paths.get(full)
                if (direct.isRegularFile()) listOf(direct.toString()) else emptyList()
            }
        }
    }
}

/** Builds a [Host] from a config entry, or null for wildcard patterns. */
fun SshConfigEntry.toHostOrNull(): app.maine.android.model.Host? {
    if (!SshConfigEntry.isConnectable(pattern)) return null
    return app.maine.android.model.Host(
        name = pattern,
        hostname = pattern,
        port = port ?: app.maine.android.model.Host.DEFAULT_PORT,
        username = user ?: "",
    )
}
