package dev.ladoflow.display.protocol

const val VIDEO_FRAME_METADATA_BYTES: Int = 28
const val MAX_ENCODED_VIDEO_BYTES: Int = LDFL_MAX_MEDIA_PAYLOAD_BYTES - VIDEO_FRAME_METADATA_BYTES

private const val DISPLAY_CONFIG_BYTES = 14

enum class VideoCodec(val wireValue: Int) {
    H264(1),
    Hevc(2),
    Av1(3),
    ;

    companion object {
        fun fromWire(value: Int): VideoCodec = entries.firstOrNull { it.wireValue == value }
            ?: protocolFailure(ProtocolViolation.InvalidPayload, "Unknown video codec $value")
    }
}

enum class CodecProfile(
    val wireValue: Int,
    val codec: VideoCodec,
) {
    H264Baseline(1, VideoCodec.H264),
    H264Main(2, VideoCodec.H264),
    H264High(3, VideoCodec.H264),
    HevcMain(16, VideoCodec.Hevc),
    HevcMain10(17, VideoCodec.Hevc),
    Av1Main(32, VideoCodec.Av1),
    ;

    companion object {
        fun fromWire(value: Int): CodecProfile = entries.firstOrNull { it.wireValue == value }
            ?: protocolFailure(ProtocolViolation.InvalidPayload, "Unknown codec profile $value")
    }
}

data class DisplayConfigPayload(
    val width: Int,
    val height: Int,
    val refreshMillihz: UInt,
    val bitrateKbps: UInt,
    val codec: VideoCodec,
    val profile: CodecProfile,
) : LdflPayload {
    override val messageType: MessageType = MessageType.DisplayConfig

    init {
        requireProtocol(width in 1..0xffff && height in 1..0xffff) {
            "Display dimensions must be non-zero u16 values"
        }
        requireProtocol(refreshMillihz != 0u) { "Display refresh rate must be non-zero" }
        requireProtocol(bitrateKbps != 0u) { "Display bitrate must be non-zero" }
        requireProtocol(profile.codec == codec) { "Codec profile does not belong to selected codec" }
    }

    override fun encode(): ByteArray {
        val writer = NetworkByteWriter(DISPLAY_CONFIG_BYTES)
        writer.u16(width)
        writer.u16(height)
        writer.u32(refreshMillihz)
        writer.u32(bitrateKbps)
        writer.u8(codec.wireValue)
        writer.u8(profile.wireValue)
        return writer.toByteArray()
    }

    companion object {
        fun decode(payload: ByteArray): DisplayConfigPayload {
            requireProtocol(payload.size == DISPLAY_CONFIG_BYTES) {
                "Display-config payload must be exactly $DISPLAY_CONFIG_BYTES bytes"
            }
            return DisplayConfigPayload(
                width = payload.readU16(0),
                height = payload.readU16(2),
                refreshMillihz = payload.readU32(4),
                bitrateKbps = payload.readU32(8),
                codec = VideoCodec.fromWire(payload.readU8(12)),
                profile = CodecProfile.fromWire(payload.readU8(13)),
            )
        }
    }
}

data class VideoFrameMetadata(
    val frameId: ULong,
    val captureTimestampMicros: ULong,
    val presentationTimestampMicros: ULong,
    val durationMicros: UInt,
) {
    init {
        requireProtocol(durationMicros != 0u) { "Video frame duration must be non-zero" }
    }

    internal fun encodeInto(writer: NetworkByteWriter) {
        writer.u64(frameId)
        writer.u64(captureTimestampMicros)
        writer.u64(presentationTimestampMicros)
        writer.u32(durationMicros)
    }

    companion object {
        internal fun decode(payload: ByteArray): VideoFrameMetadata = VideoFrameMetadata(
            frameId = payload.readU64(0),
            captureTimestampMicros = payload.readU64(8),
            presentationTimestampMicros = payload.readU64(16),
            durationMicros = payload.readU32(24),
        )
    }
}

class VideoFramePayload(
    val metadata: VideoFrameMetadata,
    accessUnit: ByteArray,
) : LdflPayload {
    val accessUnit: ByteArray = accessUnit.copyOf()

    override val messageType: MessageType = MessageType.VideoFrame

    init {
        validateAccessUnit(this.accessUnit)
    }

    override fun encode(): ByteArray {
        validateAccessUnit(accessUnit)
        val writer = NetworkByteWriter(VIDEO_FRAME_METADATA_BYTES + accessUnit.size)
        metadata.encodeInto(writer)
        writer.bytes(accessUnit)
        return writer.toByteArray()
    }

    override fun equals(other: Any?): Boolean =
        other is VideoFramePayload && metadata == other.metadata && accessUnit.contentEquals(other.accessUnit)

    override fun hashCode(): Int = 31 * metadata.hashCode() + accessUnit.contentHashCode()

    companion object {
        fun decode(payload: ByteArray): VideoFramePayload {
            requireProtocol(
                payload.size <= LDFL_MAX_MEDIA_PAYLOAD_BYTES,
                ProtocolViolation.PayloadTooLarge,
            ) { "VideoFrame payload exceeds the 16 MiB limit" }
            requireProtocol(payload.size > VIDEO_FRAME_METADATA_BYTES) {
                "Video-frame payload is missing metadata or encoded bytes"
            }
            return VideoFramePayload(
                metadata = VideoFrameMetadata.decode(payload),
                accessUnit = payload.copyOfRange(VIDEO_FRAME_METADATA_BYTES, payload.size),
            )
        }

        private fun validateAccessUnit(bytes: ByteArray) {
            requireProtocol(bytes.isNotEmpty()) { "Encoded video access unit must not be empty" }
            requireProtocol(
                bytes.size <= MAX_ENCODED_VIDEO_BYTES,
                ProtocolViolation.PayloadTooLarge,
            ) { "Encoded video access unit exceeds the version-one media limit" }
        }
    }
}
