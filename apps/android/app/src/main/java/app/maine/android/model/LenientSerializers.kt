package app.maine.android.model

import kotlinx.serialization.KSerializer
import kotlinx.serialization.SerializationException
import kotlinx.serialization.descriptors.PrimitiveKind
import kotlinx.serialization.descriptors.PrimitiveSerialDescriptor
import kotlinx.serialization.descriptors.SerialDescriptor
import kotlinx.serialization.encoding.Decoder
import kotlinx.serialization.encoding.Encoder
import kotlinx.serialization.json.JsonDecoder
import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.JsonPrimitive

/**
 * Swift-like lenient field decoding for [Host]: a value that is absent or
 * explicitly null falls back to its default, while a value with the wrong
 * JSON type fails the whole file. (kotlinx's `explicitNulls = false` does
 * not skip nulls on decode, so the fallback lives here instead.)
 */

/** `"name"`, `"hostname"`, `"username"`: null means `""`. */
internal object LenientString : KSerializer<String> {
    override val descriptor: SerialDescriptor =
        PrimitiveSerialDescriptor("LenientString", PrimitiveKind.STRING)

    override fun serialize(encoder: Encoder, value: String) = encoder.encodeString(value)

    override fun deserialize(decoder: Decoder): String {
        if (decoder is JsonDecoder) {
            val element = decoder.decodeJsonElement()
            if (element is JsonNull) return ""
            val primitive = element as? JsonPrimitive ?: throw SerializationException(
                "\"hosts\" entries should hold strings.",
            )
            if (!primitive.isString) throw SerializationException(
                "\"hosts\" entries should hold strings.",
            )
            return primitive.content
        }
        return decoder.decodeString()
    }
}

/** `"port"`: null means 22. A JSON string is not a port, even if numeric. */
internal object LenientPort : KSerializer<Int> {
    override val descriptor: SerialDescriptor =
        PrimitiveSerialDescriptor("LenientPort", PrimitiveKind.INT)

    override fun serialize(encoder: Encoder, value: Int) = encoder.encodeInt(value)

    override fun deserialize(decoder: Decoder): Int {
        if (decoder is JsonDecoder) {
            val element = decoder.decodeJsonElement()
            if (element is JsonNull) return Host.DEFAULT_PORT
            val primitive = element as? JsonPrimitive ?: throw SerializationException(
                "\"hosts.port\" should hold an integer.",
            )
            if (primitive.isString) throw SerializationException(
                "\"hosts.port\" should hold an integer.",
            )
            return primitive.content.toIntOrNull() ?: throw SerializationException(
                "\"hosts.port\" should hold an integer.",
            )
        }
        return decoder.decodeInt()
    }
}

/** `"id"`: null means a fresh UUID, exactly like the macOS reader.
 * A present value is kept as-is and validated by [HostConfiguration]. */
internal object LenientId : KSerializer<String> {
    override val descriptor: SerialDescriptor =
        PrimitiveSerialDescriptor("LenientId", PrimitiveKind.STRING)

    override fun serialize(encoder: Encoder, value: String) = encoder.encodeString(value)

    override fun deserialize(decoder: Decoder): String {
        if (decoder is JsonDecoder) {
            val element = decoder.decodeJsonElement()
            if (element is JsonNull) return Host.newId()
            val primitive = element as? JsonPrimitive ?: throw SerializationException(
                "\"hosts.id\" should hold a UUID.",
            )
            if (!primitive.isString) throw SerializationException(
                "\"hosts.id\" should hold a UUID.",
            )
            return primitive.content
        }
        return decoder.decodeString()
    }
}
