package dev.ladoflow.display.protocol

private const val PING_BYTES = 16
private const val PONG_BYTES = 32
private const val ERROR_PREFIX_BYTES = 5
private const val MAX_ERROR_DIAGNOSTIC_BYTES = 1_024

data class PingPayload(
    val token: ULong,
    val clientSendTimestampMicros: ULong,
) : LdflPayload {
    override val messageType: MessageType = MessageType.Ping

    override fun encode(): ByteArray {
        val writer = NetworkByteWriter(PING_BYTES)
        writer.u64(token)
        writer.u64(clientSendTimestampMicros)
        return writer.toByteArray()
    }

    companion object {
        fun decode(payload: ByteArray): PingPayload {
            requireProtocol(payload.size == PING_BYTES) {
                "Ping payload must be exactly $PING_BYTES bytes"
            }
            return PingPayload(payload.readU64(0), payload.readU64(8))
        }
    }
}

data class PongPayload(
    val token: ULong,
    val clientSendTimestampMicros: ULong,
    val serverReceiveTimestampMicros: ULong,
    val serverSendTimestampMicros: ULong,
) : LdflPayload {
    override val messageType: MessageType = MessageType.Pong

    init {
        requireProtocol(serverSendTimestampMicros >= serverReceiveTimestampMicros) {
            "Pong server send timestamp precedes receive timestamp"
        }
    }

    override fun encode(): ByteArray {
        val writer = NetworkByteWriter(PONG_BYTES)
        writer.u64(token)
        writer.u64(clientSendTimestampMicros)
        writer.u64(serverReceiveTimestampMicros)
        writer.u64(serverSendTimestampMicros)
        return writer.toByteArray()
    }

    companion object {
        fun decode(payload: ByteArray): PongPayload {
            requireProtocol(payload.size == PONG_BYTES) {
                "Pong payload must be exactly $PONG_BYTES bytes"
            }
            return PongPayload(
                token = payload.readU64(0),
                clientSendTimestampMicros = payload.readU64(8),
                serverReceiveTimestampMicros = payload.readU64(16),
                serverSendTimestampMicros = payload.readU64(24),
            )
        }
    }
}

enum class RemoteErrorCode(val wireValue: Int) {
    ProtocolViolation(1),
    Unsupported(2),
    ConfigurationRejected(3),
    Unauthorized(4),
    Busy(5),
    EncoderFailure(6),
    DecoderFailure(7),
    InputRejected(8),
    ResourceExhausted(9),
    Internal(10),
    ;

    companion object {
        fun fromWire(value: Int): RemoteErrorCode = entries.firstOrNull { it.wireValue == value }
            ?: protocolFailure(
                dev.ladoflow.display.protocol.ProtocolViolation.InvalidPayload,
                "Unknown remote error code $value",
            )
    }
}

data class RemoteErrorPayload(
    val code: RemoteErrorCode,
    val retryable: Boolean,
    val diagnostic: String,
) : LdflPayload {
    override val messageType: MessageType = MessageType.Error

    init {
        validateDiagnostic(diagnostic)
    }

    override fun encode(): ByteArray {
        val diagnosticBytes = diagnostic.toByteArray(Charsets.UTF_8)
        val writer = NetworkByteWriter(ERROR_PREFIX_BYTES + diagnosticBytes.size)
        writer.u16(code.wireValue)
        writer.u8(if (retryable) 1 else 0)
        writer.u16(diagnosticBytes.size)
        writer.bytes(diagnosticBytes)
        return writer.toByteArray()
    }

    companion object {
        fun decode(payload: ByteArray): RemoteErrorPayload {
            requireProtocol(payload.size >= ERROR_PREFIX_BYTES) { "Error payload is truncated" }
            val diagnosticLength = payload.readU16(3)
            requireProtocol(diagnosticLength <= MAX_ERROR_DIAGNOSTIC_BYTES) {
                "Error diagnostic exceeds $MAX_ERROR_DIAGNOSTIC_BYTES UTF-8 bytes"
            }
            requireProtocol(payload.size == ERROR_PREFIX_BYTES + diagnosticLength) {
                "Error diagnostic length does not match payload"
            }
            return RemoteErrorPayload(
                code = RemoteErrorCode.fromWire(payload.readU16(0)),
                retryable = decodeBoolean(payload.readU8(2)),
                diagnostic = payload.copyOfRange(ERROR_PREFIX_BYTES, payload.size).decodeStrictUtf8(),
            )
        }

        private fun decodeBoolean(value: Int): Boolean = when (value) {
            0 -> false
            1 -> true
            else -> protocolFailure(
                ProtocolViolation.InvalidPayload,
                "Error retryable field must be zero or one",
            )
        }

        private fun validateDiagnostic(diagnostic: String) {
            val bytes = diagnostic.toByteArray(Charsets.UTF_8)
            requireProtocol(bytes.size <= MAX_ERROR_DIAGNOSTIC_BYTES) {
                "Error diagnostic exceeds $MAX_ERROR_DIAGNOSTIC_BYTES UTF-8 bytes"
            }
            requireProtocol('\u0000' !in diagnostic) { "Error diagnostic contains a null byte" }
        }
    }
}
