package dev.ladoflow.display.protocol

private const val HELLO_PREFIX_BYTES = 22
private const val MAX_IMPLEMENTATION_NAME_BYTES = 64
private const val CAPABILITIES_BYTES = 20

enum class EndpointRole(val wireValue: Int) {
    Host(1),
    Display(2),
    ;

    companion object {
        fun fromWire(value: Int): EndpointRole = entries.firstOrNull { it.wireValue == value }
            ?: protocolFailure(ProtocolViolation.InvalidPayload, "Unknown endpoint role $value")
    }
}

@ConsistentCopyVisibility
data class CodecCapabilities private constructor(val bits: Int) {
    fun contains(other: CodecCapabilities): Boolean = bits and other.bits == other.bits

    infix fun or(other: CodecCapabilities): CodecCapabilities = fromBits(bits or other.bits)

    companion object {
        val None = CodecCapabilities(0)
        val H264 = CodecCapabilities(1 shl 0)
        val Hevc = CodecCapabilities(1 shl 1)
        val Av1 = CodecCapabilities(1 shl 2)

        private const val KNOWN_MASK = (1 shl 0) or (1 shl 1) or (1 shl 2)

        fun fromBits(bits: Int): CodecCapabilities {
            requireProtocol(bits and KNOWN_MASK.inv() == 0) {
                "Unknown codec capability bits 0x${bits.toString(16)}"
            }
            return CodecCapabilities(bits)
        }
    }
}

@ConsistentCopyVisibility
data class InputCapabilities private constructor(val bits: Int) {
    fun contains(other: InputCapabilities): Boolean = bits and other.bits == other.bits

    infix fun or(other: InputCapabilities): InputCapabilities = fromBits(bits or other.bits)

    companion object {
        val None = InputCapabilities(0)
        val Pointer = InputCapabilities(1 shl 0)
        val Touch = InputCapabilities(1 shl 1)
        val Keyboard = InputCapabilities(1 shl 2)

        private const val KNOWN_MASK = (1 shl 0) or (1 shl 1) or (1 shl 2)

        fun fromBits(bits: Int): InputCapabilities {
            requireProtocol(bits and KNOWN_MASK.inv() == 0) {
                "Unknown input capability bits 0x${bits.toString(16)}"
            }
            return InputCapabilities(bits)
        }
    }
}

@ConsistentCopyVisibility
data class FeatureFlags private constructor(val bits: UInt) {
    fun contains(other: FeatureFlags): Boolean = bits and other.bits == other.bits

    infix fun or(other: FeatureFlags): FeatureFlags = fromBits(bits or other.bits)

    companion object {
        val None = FeatureFlags(0u)
        val DynamicRotation = FeatureFlags(1u shl 0)
        val RemoteCursor = FeatureFlags(1u shl 1)
        val Audio = FeatureFlags(1u shl 2)

        private val KNOWN_MASK = (1u shl 0) or (1u shl 1) or (1u shl 2)

        fun fromBits(bits: UInt): FeatureFlags {
            requireProtocol(bits and KNOWN_MASK.inv() == 0u) {
                "Unknown feature capability bits 0x${bits.toString(16)}"
            }
            return FeatureFlags(bits)
        }
    }
}

class HelloPayload(
    val minProtocol: Int,
    val maxProtocol: Int,
    val role: EndpointRole,
    nonce: ByteArray,
    val implementationName: String,
) : LdflPayload {
    val nonce: ByteArray = nonce.copyOf()

    override val messageType: MessageType = MessageType.Hello

    init {
        requireProtocol(minProtocol in 1..0xffff) { "Minimum protocol version must be non-zero" }
        requireProtocol(maxProtocol in minProtocol..0xffff) {
            "Minimum protocol version exceeds maximum"
        }
        requireProtocol(this.nonce.size == 16) { "Hello nonce must be exactly 16 bytes" }
        validateImplementationName(implementationName)
    }

    override fun encode(): ByteArray {
        val nameBytes = implementationName.toByteArray(Charsets.UTF_8)
        val writer = NetworkByteWriter(HELLO_PREFIX_BYTES + nameBytes.size)
        writer.u16(minProtocol)
        writer.u16(maxProtocol)
        writer.u8(role.wireValue)
        writer.u8(nameBytes.size)
        writer.bytes(nonce)
        writer.bytes(nameBytes)
        return writer.toByteArray()
    }

    override fun equals(other: Any?): Boolean =
        other is HelloPayload &&
            minProtocol == other.minProtocol &&
            maxProtocol == other.maxProtocol &&
            role == other.role &&
            nonce.contentEquals(other.nonce) &&
            implementationName == other.implementationName

    override fun hashCode(): Int {
        var result = minProtocol
        result = 31 * result + maxProtocol
        result = 31 * result + role.hashCode()
        result = 31 * result + nonce.contentHashCode()
        result = 31 * result + implementationName.hashCode()
        return result
    }

    companion object {
        fun decode(payload: ByteArray): HelloPayload {
            requireProtocol(payload.size >= HELLO_PREFIX_BYTES) { "Hello payload is truncated" }
            val nameLength = payload.readU8(5)
            requireProtocol(payload.size == HELLO_PREFIX_BYTES + nameLength) {
                "Hello implementation-name length does not match payload"
            }
            return HelloPayload(
                minProtocol = payload.readU16(0),
                maxProtocol = payload.readU16(2),
                role = EndpointRole.fromWire(payload.readU8(4)),
                nonce = payload.copyOfRange(6, 22),
                implementationName = payload.copyOfRange(22, payload.size).decodeStrictUtf8(),
            )
        }

        private fun validateImplementationName(name: String) {
            val bytes = name.toByteArray(Charsets.UTF_8)
            requireProtocol(bytes.isNotEmpty()) { "Implementation name must not be empty" }
            requireProtocol(bytes.size <= MAX_IMPLEMENTATION_NAME_BYTES) {
                "Implementation name exceeds $MAX_IMPLEMENTATION_NAME_BYTES UTF-8 bytes"
            }
            requireProtocol('\u0000' !in name) { "Implementation name contains a null byte" }
        }
    }
}

data class CapabilitiesPayload(
    val maxWidth: Int,
    val maxHeight: Int,
    val maxRefreshMillihz: UInt,
    val maxBitrateKbps: UInt,
    val codecs: CodecCapabilities,
    val input: InputCapabilities,
    val features: FeatureFlags,
) : LdflPayload {
    override val messageType: MessageType = MessageType.Capabilities

    init {
        requireProtocol(maxWidth in 1..0xffff && maxHeight in 1..0xffff) {
            "Maximum display dimensions must be non-zero u16 values"
        }
        requireProtocol(maxRefreshMillihz != 0u) { "Maximum refresh rate must be non-zero" }
        requireProtocol(maxBitrateKbps != 0u) { "Maximum bitrate must be non-zero" }
        requireProtocol(codecs != CodecCapabilities.None) { "At least one codec must be supported" }
    }

    override fun encode(): ByteArray {
        val writer = NetworkByteWriter(CAPABILITIES_BYTES)
        writer.u16(maxWidth)
        writer.u16(maxHeight)
        writer.u32(maxRefreshMillihz)
        writer.u32(maxBitrateKbps)
        writer.u16(codecs.bits)
        writer.u16(input.bits)
        writer.u32(features.bits)
        return writer.toByteArray()
    }

    companion object {
        fun decode(payload: ByteArray): CapabilitiesPayload {
            requireProtocol(payload.size == CAPABILITIES_BYTES) {
                "Capabilities payload must be exactly $CAPABILITIES_BYTES bytes"
            }
            return CapabilitiesPayload(
                maxWidth = payload.readU16(0),
                maxHeight = payload.readU16(2),
                maxRefreshMillihz = payload.readU32(4),
                maxBitrateKbps = payload.readU32(8),
                codecs = CodecCapabilities.fromBits(payload.readU16(12)),
                input = InputCapabilities.fromBits(payload.readU16(14)),
                features = FeatureFlags.fromBits(payload.readU32(16)),
            )
        }
    }
}
