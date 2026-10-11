package app.maine.android.model

import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable
import kotlinx.serialization.SerializationException
import kotlinx.serialization.json.Json

/**
 * The on-disk host list: the exact JSON Maine writes.
 *
 * The format is defined by `packages/schema/hosts.schema.json`. This type is
 * used directly for legacy/plain files and to re-validate hosts decrypted
 * from the encrypted vault: a host that would not load back must never save.
 */
@Serializable
data class HostConfiguration(
    @SerialName("schemaVersion")
    val schemaVersion: Int = CURRENT_SCHEMA_VERSION,
    @SerialName("hosts")
    val hosts: List<Host> = emptyList(),
) {
    companion object {
        /** The format version this build writes and understands. */
        const val CURRENT_SCHEMA_VERSION = 1

        /**
         * Lenient reading that mirrors the macOS client: a field that is
         * absent or explicitly null falls back to its default, while a field
         * with the wrong JSON type is an error. Unknown properties are
         * ignored here; schema strictness lives in `packages/schema`.
         * `coerceInputValues` stays off so mistyped fields fail loudly.
         */
        val lenientJson = Json {
            ignoreUnknownKeys = true
            explicitNulls = false
        }

        @Throws(DecodeError::class)
        fun decode(data: String): HostConfiguration {
            val configuration = try {
                lenientJson.decodeFromString(serializer(), data)
            } catch (e: SerializationException) {
                throw DecodeError.Malformed(e.message ?: "the file is not valid JSON.")
            } catch (e: IllegalArgumentException) {
                throw DecodeError.Malformed(e.message ?: "the file is not valid JSON.")
            }
            if (configuration.schemaVersion > CURRENT_SCHEMA_VERSION) {
                throw DecodeError.UnsupportedVersion(configuration.schemaVersion)
            }
            // IDs must be UUIDs, matching the macOS vault validation.
            for (host in configuration.hosts) {
                try {
                    Host.normalizeId(host.id)
                } catch (e: IllegalArgumentException) {
                    throw DecodeError.Malformed("\"hosts.id\" should hold a UUID.")
                }
            }
            return configuration
        }
    }

    /** This build understands the file when its version is not newer. */
    val isSupportedByThisVersion: Boolean get() = schemaVersion <= CURRENT_SCHEMA_VERSION

    fun encoded(): String = Json { prettyPrint = true }.encodeToString(serializer(), this)

    sealed class DecodeError(message: String) : Exception(message) {
        /** Written by a newer client. Must not load, must not overwrite. */
        class UnsupportedVersion(version: Int) : DecodeError(
            "This host list was written with a newer version of the Maine " +
                "configuration format (version $version). Update Maine to read it.",
        )

        /** Valid JSON but not a version-1 host list. */
        class Malformed(detail: String) : DecodeError(
            "Maine could not read this host list: $detail",
        )
    }
}
