package app.maine.android.model

import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable
import java.util.UUID

/**
 * A single saved SSH host.
 *
 * Maps 1:1 onto the `host` object defined by
 * `packages/schema/hosts.schema.json`, the portable format shared with the
 * macOS client. Validation messages intentionally match the macOS wording.
 *
 * IDs are UUID strings. The macOS client writes them in the uppercase
 * `UUID().uuidString` form and looks passwords up with that exact casing, so
 * this client normalizes IDs to uppercase on read and writes uppercase: the
 * vault file stays interoperable in both directions. Comparisons must still
 * be case-insensitive because the schema recommends lowercase.
 */
@Serializable
data class Host(
    @Serializable(with = LenientId::class)
    @SerialName("id")
    val id: String = newId(),
    @Serializable(with = LenientString::class)
    @SerialName("name")
    val name: String = "",
    @Serializable(with = LenientString::class)
    @SerialName("hostname")
    val hostname: String = "",
    @Serializable(with = LenientPort::class)
    @SerialName("port")
    val port: Int = DEFAULT_PORT,
    @Serializable(with = LenientString::class)
    @SerialName("username")
    val username: String = "",
) {
    companion object {
        /** Storing 22 means "default": the client omits `-p` so server-side
         * config can pick the real port. */
        const val DEFAULT_PORT = 22

        fun newId(): String = UUID.randomUUID().toString().uppercase()

        /** Normalizes any UUID string to the canonical uppercase form.
         * Throws [IllegalArgumentException] when it is not a UUID. */
        fun normalizeId(raw: String): String =
            UUID.fromString(raw.trim()).toString().uppercase()

        fun idsEqual(a: String, b: String): Boolean =
            runCatching { UUID.fromString(a.trim()) == UUID.fromString(b.trim()) }
                .getOrDefault(a.trim().equals(b.trim(), ignoreCase = true))
    }

    /** The trimmed name used for display. */
    val displayName: String
        get() {
            val trimmed = name.trim()
            return if (trimmed.isEmpty()) hostname else trimmed
        }

    /** `user@hostname`, or just `hostname` when no user is set. */
    val target: String
        get() = if (username.isEmpty()) hostname else "$username@$hostname"

    /** The port, only shown when it is not the default. */
    val portDescription: String?
        get() = if (port == DEFAULT_PORT) null else ":$port"

    /** A one line summary used in the host list and session headers. */
    val subtitle: String
        get() {
            var line = target
            if (portDescription != null) line += portDescription
            return line
        }

    /**
     * Problems that stop this host from being used. Mirrors the constraints
     * in the shared schema and the macOS client's checks exactly, including
     * the gaps (e.g. name length above 128 is not rejected there either).
     */
    val issues: List<String>
        get() {
            val found = mutableListOf<String>()
            if (name.trim().isEmpty()) {
                found += "A name is required."
            }
            val address = hostname.trim()
            if (address.isEmpty()) {
                found += "A hostname or IP address is required."
            } else if (address.any { it.isWhitespace() }) {
                found += "The hostname cannot contain spaces."
            } else if (address.startsWith("-")) {
                found += "The hostname cannot start with \"-\", it would be read as an option by ssh."
            } else if (address.length > 253) {
                found += "The hostname cannot be longer than 253 characters."
            }
            if (port !in 1..65535) {
                found += "The port must be between 1 and 65535."
            }
            if (username.any { it.isWhitespace() }) {
                found += "The username cannot contain spaces."
            } else if (username.length > 128) {
                found += "The username cannot be longer than 128 characters."
            }
            return found
        }

    val isValid: Boolean get() = issues.isEmpty()
}
