package app.maine.android.vault

import kotlinx.serialization.KSerializer
import kotlinx.serialization.SerializationException
import kotlinx.serialization.descriptors.PrimitiveKind
import kotlinx.serialization.descriptors.PrimitiveSerialDescriptor
import kotlinx.serialization.descriptors.SerialDescriptor
import kotlinx.serialization.encoding.Decoder
import kotlinx.serialization.encoding.Encoder
import java.util.Base64

/**
 * Swift `Data` JSON encoding: a single standard-base64 string.
 *
 * kotlinx.serialization's default `ByteArray` encoding is a JSON array of
 * numbers, which would make our files unreadable to the macOS client (and
 * vice versa). Every `ByteArray` in the vault envelope uses this instead,
 * so the bytes on disk are identical on both clients.
 */
internal object Base64Bytes : KSerializer<ByteArray> {
    override val descriptor: SerialDescriptor =
        PrimitiveSerialDescriptor("Base64Bytes", PrimitiveKind.STRING)

    override fun serialize(encoder: Encoder, value: ByteArray) {
        encoder.encodeString(Base64.getEncoder().encodeToString(value))
    }

    override fun deserialize(decoder: Decoder): ByteArray {
        val raw = decoder.decodeString()
        try {
            return Base64.getDecoder().decode(raw)
        } catch (e: IllegalArgumentException) {
            throw SerializationException("Expected standard-base64 data, got \"$raw\".")
        }
    }
}
