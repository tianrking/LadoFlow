package dev.ladoflow.display.protocol

const val LDFL_PROTOCOL_VERSION: Int = 1
const val LDFL_FRAME_HEADER_BYTES: Int = 24
const val LDFL_MAX_CONTROL_PAYLOAD_BYTES: Int = 64 * 1024
const val LDFL_MAX_MEDIA_PAYLOAD_BYTES: Int = 16 * 1024 * 1024
const val LDFL_DEFAULT_BUFFER_LIMIT_BYTES: Int =
    2 * (LDFL_FRAME_HEADER_BYTES + LDFL_MAX_MEDIA_PAYLOAD_BYTES)

enum class ProtocolViolation {
    InvalidMagic,
    InvalidHeaderLength,
    UnsupportedVersion,
    UnknownMessageType,
    UnknownFrameFlags,
    PayloadTooLarge,
    BufferLimitExceeded,
    UnexpectedMessageType,
    NonMonotonicSequence,
    InvalidPayload,
    InvalidUtf8,
}

/** One validator per physical/session stream; sequence zero is valid. */
class MonotonicSequenceValidator {
    private var highest: ULong? = null

    fun observe(sequence: ULong) {
        val previous = highest
        requireProtocol(
            previous == null || sequence > previous,
            ProtocolViolation.NonMonotonicSequence,
        ) {
            "LDFL sequence $sequence is duplicate or stale after $previous"
        }
        highest = sequence
    }
}

class LdflProtocolException(
    val violation: ProtocolViolation,
    message: String,
) : IllegalArgumentException(message)

enum class MessageType(
    val wireValue: Int,
    val payloadLimit: Int,
) {
    Hello(1, LDFL_MAX_CONTROL_PAYLOAD_BYTES),
    Capabilities(2, LDFL_MAX_CONTROL_PAYLOAD_BYTES),
    DisplayConfig(3, LDFL_MAX_CONTROL_PAYLOAD_BYTES),
    VideoFrame(4, LDFL_MAX_MEDIA_PAYLOAD_BYTES),
    Input(5, LDFL_MAX_CONTROL_PAYLOAD_BYTES),
    Telemetry(6, LDFL_MAX_CONTROL_PAYLOAD_BYTES),
    Ping(7, LDFL_MAX_CONTROL_PAYLOAD_BYTES),
    Pong(8, LDFL_MAX_CONTROL_PAYLOAD_BYTES),
    Error(9, LDFL_MAX_CONTROL_PAYLOAD_BYTES),
    ;

    companion object {
        fun fromWire(value: Int): MessageType = entries.firstOrNull { it.wireValue == value }
            ?: protocolFailure(
                ProtocolViolation.UnknownMessageType,
                "Unknown LDFL message type $value",
            )
    }
}

@ConsistentCopyVisibility
data class FrameFlags private constructor(val bits: Int) {
    init {
        if (bits and KNOWN_MASK.inv() != 0) {
            protocolFailure(
                ProtocolViolation.UnknownFrameFlags,
                "Unknown LDFL frame flags 0x${bits.toString(16).padStart(4, '0')}",
            )
        }
    }

    fun contains(other: FrameFlags): Boolean = bits and other.bits == other.bits

    infix fun or(other: FrameFlags): FrameFlags = fromBits(bits or other.bits)

    companion object {
        val None = FrameFlags(0)
        val Keyframe = FrameFlags(1 shl 0)
        val EndOfStream = FrameFlags(1 shl 1)
        val AckRequired = FrameFlags(1 shl 2)

        private const val KNOWN_MASK: Int = (1 shl 0) or (1 shl 1) or (1 shl 2)

        fun fromBits(bits: Int): FrameFlags = FrameFlags(bits)
    }
}

sealed interface LdflPayload {
    val messageType: MessageType

    fun encode(): ByteArray
}

@PublishedApi
internal fun protocolFailure(
    violation: ProtocolViolation,
    message: String,
): Nothing = throw LdflProtocolException(violation, message)

internal fun requireProtocol(
    condition: Boolean,
    violation: ProtocolViolation = ProtocolViolation.InvalidPayload,
    message: () -> String,
) {
    if (!condition) {
        protocolFailure(violation, message())
    }
}
