package dev.ladoflow.display.protocol

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Test

class FrameCodecTest {
    @Test
    fun `frame layout matches the Rust big-endian golden vector`() {
        val frame = LdflFrame(
            messageType = MessageType.Ping,
            flags = FrameFlags.AckRequired,
            sequence = 0x0102_0304_0506_0708uL,
            payload = byteArrayOf(0xaa.toByte(), 0xbb.toByte()),
        )

        assertEquals(
            "4c44464c0001001800070004010203040506070800000002aabb",
            frame.encode().toHex(),
        )

        val decoded = FrameCodec.decodePrefix(frame.encode()) as FrameDecodeResult.Complete
        assertEquals(frame, decoded.frame)
        assertEquals(frame.encode().size, decoded.consumed)
    }

    @Test
    fun `partial header and payload report exact required prefix`() {
        val frame = LdflFrame(MessageType.Pong, FrameFlags.None, 7uL, byteArrayOf(1, 2, 3, 4))
        val encoded = frame.encode()

        assertEquals(
            FrameDecodeResult.NeedMoreData(LDFL_FRAME_HEADER_BYTES),
            FrameCodec.decodePrefix(encoded.copyOf(7)),
        )
        assertEquals(
            FrameDecodeResult.NeedMoreData(encoded.size),
            FrameCodec.decodePrefix(encoded.copyOf(LDFL_FRAME_HEADER_BYTES + 2)),
        )
    }

    @Test
    fun `incremental decoder accepts every one-byte split`() {
        val expected = LdflFrame(
            MessageType.Input,
            FrameFlags.AckRequired,
            99uL,
            "pointer".toByteArray(),
        )
        val decoder = IncrementalFrameDecoder()
        val frames = mutableListOf<LdflFrame>()

        expected.encode().forEach { byte -> frames += decoder.push(byteArrayOf(byte)) }

        assertEquals(listOf(expected), frames)
        assertEquals(0, decoder.bufferedBytes)
    }

    @Test
    fun `incremental decoder emits coalesced frames and retains a partial tail`() {
        val first = LdflFrame.fromPayload(FrameFlags.None, 10uL, PingPayload(1uL, 2uL))
        val second = LdflFrame.fromPayload(
            FrameFlags.None,
            11uL,
            PongPayload(1uL, 2uL, 3uL, 4uL),
        )
        val third = LdflFrame.fromPayload(FrameFlags.Keyframe, 12uL, videoPayload())
        val stream = first.encode() + second.encode() + third.encode()
        val split = stream.size - 5
        val decoder = IncrementalFrameDecoder()

        assertEquals(listOf(first, second), decoder.push(stream.copyOf(split)))
        assertEquals(third, decoder.push(stream.copyOfRange(split, stream.size)).single())
        assertEquals(0, decoder.bufferedBytes)
    }

    @Test
    fun `malformed header fields are rejected before payload allocation`() {
        val base = LdflFrame(MessageType.Ping, FrameFlags.None, 1uL, ByteArray(0)).encode()

        assertViolation(ProtocolViolation.InvalidMagic) {
            base.copyOf().also { it[0] = 'X'.code.toByte() }.decode()
        }
        assertViolation(ProtocolViolation.UnsupportedVersion) {
            base.copyOf().also { it.writeU16(4, 2) }.decode()
        }
        assertViolation(ProtocolViolation.InvalidHeaderLength) {
            base.copyOf().also { it.writeU16(6, 25) }.decode()
        }
        assertViolation(ProtocolViolation.UnknownMessageType) {
            base.copyOf().also { it.writeU16(8, 99) }.decode()
        }
        assertViolation(ProtocolViolation.UnknownFrameFlags) {
            base.copyOf().also { it.writeU16(10, 0x8000) }.decode()
        }
        assertViolation(ProtocolViolation.PayloadTooLarge) {
            base.copyOf().also { it.writeU32(20, (LDFL_MAX_CONTROL_PAYLOAD_BYTES + 1).toUInt()) }
                .decode()
        }
    }

    @Test
    fun `buffer ceiling failure does not mutate retained bytes`() {
        val decoder = IncrementalFrameDecoder(bufferLimit = 8)

        assertViolation(ProtocolViolation.BufferLimitExceeded) {
            decoder.push(ByteArray(9))
        }
        assertEquals(0, decoder.bufferedBytes)
    }

    private fun ByteArray.decode(): FrameDecodeResult = FrameCodec.decodePrefix(this)

    private fun videoPayload() = VideoFramePayload(
        metadata = VideoFrameMetadata(1uL, 2uL, 3uL, 16_667u),
        accessUnit = byteArrayOf(0, 0, 0, 1, 0x65),
    )
}

internal fun assertViolation(
    expected: ProtocolViolation,
    block: () -> Unit,
) {
    try {
        block()
        fail("Expected protocol violation $expected")
    } catch (exception: LdflProtocolException) {
        assertEquals(expected, exception.violation)
    }
}

internal fun ByteArray.toHex(): String = joinToString(separator = "") { byte ->
    (byte.toInt() and 0xff).toString(16).padStart(2, '0')
}

internal fun String.hexBytes(): ByteArray {
    require(length % 2 == 0)
    return chunked(2).map { it.toInt(16).toByte() }.toByteArray()
}

private fun ByteArray.writeU16(offset: Int, value: Int) {
    this[offset] = (value ushr 8).toByte()
    this[offset + 1] = value.toByte()
}

private fun ByteArray.writeU32(offset: Int, value: UInt) {
    this[offset] = (value shr 24).toByte()
    this[offset + 1] = (value shr 16).toByte()
    this[offset + 2] = (value shr 8).toByte()
    this[offset + 3] = value.toByte()
}
