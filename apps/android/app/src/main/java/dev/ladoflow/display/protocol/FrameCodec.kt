package dev.ladoflow.display.protocol

private val FRAME_MAGIC = byteArrayOf('L'.code.toByte(), 'D'.code.toByte(), 'F'.code.toByte(), 'L'.code.toByte())

class LdflFrame(
    val messageType: MessageType,
    val flags: FrameFlags,
    val sequence: ULong,
    payload: ByteArray,
) {
    val payload: ByteArray = payload.copyOf()

    init {
        requireProtocol(
            this.payload.size <= messageType.payloadLimit,
            ProtocolViolation.PayloadTooLarge,
        ) {
            "${messageType.name} payload is ${this.payload.size} bytes; " +
                "limit is ${messageType.payloadLimit} bytes"
        }
    }

    fun encode(): ByteArray = FrameCodec.encode(this)

    fun decodePayload(): LdflPayload = PayloadCodec.decode(messageType, payload)

    override fun equals(other: Any?): Boolean =
        other is LdflFrame &&
            messageType == other.messageType &&
            flags == other.flags &&
            sequence == other.sequence &&
            payload.contentEquals(other.payload)

    override fun hashCode(): Int {
        var result = messageType.hashCode()
        result = 31 * result + flags.hashCode()
        result = 31 * result + sequence.hashCode()
        result = 31 * result + payload.contentHashCode()
        return result
    }

    override fun toString(): String =
        "LdflFrame(type=$messageType, flags=$flags, sequence=$sequence, payloadBytes=${payload.size})"

    companion object {
        fun fromPayload(
            flags: FrameFlags,
            sequence: ULong,
            payload: LdflPayload,
        ): LdflFrame = LdflFrame(payload.messageType, flags, sequence, payload.encode())
    }
}

sealed interface FrameDecodeResult {
    data class Complete(
        val frame: LdflFrame,
        val consumed: Int,
    ) : FrameDecodeResult

    data class NeedMoreData(val minimum: Int) : FrameDecodeResult
}

object FrameCodec {
    fun encode(frame: LdflFrame): ByteArray {
        val writer = NetworkByteWriter(LDFL_FRAME_HEADER_BYTES + frame.payload.size)
        writer.bytes(FRAME_MAGIC)
        writer.u16(LDFL_PROTOCOL_VERSION)
        writer.u16(LDFL_FRAME_HEADER_BYTES)
        writer.u16(frame.messageType.wireValue)
        writer.u16(frame.flags.bits)
        writer.u64(frame.sequence)
        writer.u32(frame.payload.size.toUInt())
        writer.bytes(frame.payload)
        return writer.toByteArray()
    }

    fun decodePrefix(
        bytes: ByteArray,
        offset: Int = 0,
    ): FrameDecodeResult {
        require(offset in 0..bytes.size) { "offset is outside byte array" }
        val available = bytes.size - offset
        if (available < LDFL_FRAME_HEADER_BYTES) {
            return FrameDecodeResult.NeedMoreData(LDFL_FRAME_HEADER_BYTES)
        }

        for (index in FRAME_MAGIC.indices) {
            if (bytes[offset + index] != FRAME_MAGIC[index]) {
                protocolFailure(ProtocolViolation.InvalidMagic, "Invalid LDFL frame magic")
            }
        }

        val version = bytes.readU16(offset + 4)
        requireProtocol(version == LDFL_PROTOCOL_VERSION, ProtocolViolation.UnsupportedVersion) {
            "Unsupported LDFL version $version; supported version is $LDFL_PROTOCOL_VERSION"
        }

        val headerLength = bytes.readU16(offset + 6)
        requireProtocol(
            headerLength == LDFL_FRAME_HEADER_BYTES,
            ProtocolViolation.InvalidHeaderLength,
        ) { "Invalid LDFL header length $headerLength" }

        val messageType = MessageType.fromWire(bytes.readU16(offset + 8))
        val flags = FrameFlags.fromBits(bytes.readU16(offset + 10))
        val sequence = bytes.readU64(offset + 12)
        val payloadLength = bytes.readU32(offset + 20).toLong()

        requireProtocol(
            payloadLength <= messageType.payloadLimit.toLong(),
            ProtocolViolation.PayloadTooLarge,
        ) {
            "${messageType.name} payload is $payloadLength bytes; " +
                "limit is ${messageType.payloadLimit} bytes"
        }

        val totalLength = LDFL_FRAME_HEADER_BYTES + payloadLength.toInt()
        if (available < totalLength) {
            return FrameDecodeResult.NeedMoreData(totalLength)
        }

        val payloadOffset = offset + LDFL_FRAME_HEADER_BYTES
        return FrameDecodeResult.Complete(
            frame = LdflFrame(
                messageType = messageType,
                flags = flags,
                sequence = sequence,
                payload = bytes.copyOfRange(payloadOffset, payloadOffset + payloadLength.toInt()),
            ),
            consumed = totalLength,
        )
    }
}

class IncrementalFrameDecoder(
    private val bufferLimit: Int = LDFL_DEFAULT_BUFFER_LIMIT_BYTES,
) {
    private var buffer = ByteArray(0)

    init {
        require(bufferLimit >= 0) { "bufferLimit must be non-negative" }
    }

    val bufferedBytes: Int
        get() = buffer.size

    fun push(chunk: ByteArray): List<LdflFrame> {
        val attempted = buffer.size.toLong() + chunk.size.toLong()
        requireProtocol(
            attempted <= bufferLimit.toLong(),
            ProtocolViolation.BufferLimitExceeded,
        ) { "Decoder buffer would grow to $attempted bytes; limit is $bufferLimit bytes" }

        val combined = ByteArray(attempted.toInt())
        buffer.copyInto(combined)
        chunk.copyInto(combined, destinationOffset = buffer.size)
        buffer = combined

        val frames = mutableListOf<LdflFrame>()
        var consumed = 0
        while (consumed < buffer.size) {
            when (val result = FrameCodec.decodePrefix(buffer, consumed)) {
                is FrameDecodeResult.Complete -> {
                    frames += result.frame
                    consumed += result.consumed
                }

                is FrameDecodeResult.NeedMoreData -> break
            }
        }

        if (consumed > 0) {
            buffer = buffer.copyOfRange(consumed, buffer.size)
        }
        return frames
    }

    fun clear() {
        buffer = ByteArray(0)
    }
}
