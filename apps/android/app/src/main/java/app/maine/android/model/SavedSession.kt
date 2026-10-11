package app.maine.android.model

import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable

/**
 * A portable snapshot: history as scrollback, never a command replay.
 *
 * Field-for-field compatible with the macOS `SavedSession`. Dates use the
 * same encoding Swift's default `JSONEncoder` uses: `Date` encodes as seconds
 * since the Apple reference date (2001-01-01T00:00:00Z), **not** Unix epoch.
 */
@Serializable
data class SavedSession(
    @SerialName("id")
    val id: String = Host.newId(),
    @SerialName("hostID")
    val hostID: String,
    @SerialName("name")
    val name: String,
    @SerialName("history")
    val history: String = "",
    /** Seconds since 2001-01-01T00:00:00Z. See [nowReferenceSeconds]. */
    @SerialName("savedAt")
    val savedAt: Double = nowReferenceSeconds(),
    /** Null in vaults written before the flag existed; treated as default. */
    @SerialName("usesDefaultName")
    val usesDefaultName: Boolean? = null,
    @SerialName("number")
    val number: Int? = null,
) {
    companion object {
        /** Unix seconds of the Apple reference date (2001-01-01T00:00:00Z). */
        const val REFERENCE_DATE_OFFSET = 978307200.0

        fun nowReferenceSeconds(): Double =
            System.currentTimeMillis() / 1000.0 - REFERENCE_DATE_OFFSET

        /** Back to Unix seconds for display. */
        fun unixSeconds(referenceSeconds: Double): Double =
            referenceSeconds + REFERENCE_DATE_OFFSET

        private val DEFAULT_NAME_REGEX = Regex("^Session [0-9]+$")
    }

    /** True when the name was auto-generated ("Session N"). */
    val isDefaultName: Boolean
        get() = usesDefaultName ?: DEFAULT_NAME_REGEX.matches(name)
}
