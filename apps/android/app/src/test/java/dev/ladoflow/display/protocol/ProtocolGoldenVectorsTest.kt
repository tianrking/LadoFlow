package dev.ladoflow.display.protocol

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class ProtocolGoldenVectorsTest {
    @Test
    fun `hello vector matches the version-one field layout`() {
        val hello = HelloPayload(
            minProtocol = 1,
            maxProtocol = 1,
            role = EndpointRole.Display,
            nonce = ByteArray(16) { it.toByte() },
            implementationName = "LadoFlow Android",
        )
        val golden =
            "000100010210000102030405060708090a0b0c0d0e0f" +
                "4c61646f466c6f7720416e64726f6964"

        assertEquals(golden, hello.encode().toHex())
        assertEquals(hello, HelloPayload.decode(golden.hexBytes()))
    }

    @Test
    fun `display config vector matches the Rust crate`() {
        val config = DisplayConfigPayload(
            width = 0x1234,
            height = 0x2345,
            refreshMillihz = 0x0102_0304u,
            bitrateKbps = 0x1112_1314u,
            codec = VideoCodec.Hevc,
            profile = CodecProfile.HevcMain10,
        )

        assertEquals("1234234501020304111213140211", config.encode().toHex())
        assertEquals(config, DisplayConfigPayload.decode(config.encode()))
    }

    @Test
    fun `video frame vector matches the Rust crate`() {
        val video = VideoFramePayload(
            metadata = VideoFrameMetadata(
                frameId = 0x0102_0304_0506_0708uL,
                captureTimestampMicros = 0x1112_1314_1516_1718uL,
                presentationTimestampMicros = 0x2122_2324_2526_2728uL,
                durationMicros = 0x3132_3334u,
            ),
            accessUnit = byteArrayOf(0xaa.toByte(), 0xbb.toByte(), 0xcc.toByte()),
        )
        val golden =
            "0102030405060708" +
                "1112131415161718" +
                "2122232425262728" +
                "31323334" +
                "aabbcc"

        assertEquals(golden, video.encode().toHex())
        assertEquals(video, VideoFramePayload.decode(golden.hexBytes()))
    }

    @Test
    fun `input key vector matches the Rust crate`() {
        val key = InputPayload(
            timestampMicros = 103uL,
            event = KeyInput(
                usage = 0x04,
                state = ButtonState.Released,
                modifiers = KeyModifiers.Shift or KeyModifiers.Control,
            ),
        )

        assertEquals("0000000000000067040004000003", key.encode().toHex())
        assertEquals(key, InputPayload.decode(key.encode()))
    }

    @Test
    fun `telemetry vector preserves every network-order field`() {
        val telemetry = TelemetryPayload(
            sampleTimestampMicros = 0x0102_0304_0506_0708uL,
            frameId = 0x1112_1314_1516_1718uL,
            timings = StageTimings(10u, 20u, 30u, 40u, 50u),
            queueDepth = 7,
            lossPartsPerMillion = 125_000u,
            droppedFrames = 8u,
            lateFrames = 9u,
            thermalState = ThermalState.Fair,
        )
        val golden =
            "01020304050607081112131415161718" +
                "0000000a000000140000001e0000002800000032" +
                "00070001e848000000080000000902"

        assertEquals(golden, telemetry.encode().toHex())
        assertEquals(telemetry, TelemetryPayload.decode(golden.hexBytes()))
    }

    @Test
    fun `frame carrying typed video preserves keyframe and sequence`() {
        val video = VideoFramePayload(
            VideoFrameMetadata(1uL, 2uL, 3uL, 16_667u),
            byteArrayOf(0, 0, 0, 1, 0x65),
        )
        val frame = LdflFrame.fromPayload(FrameFlags.Keyframe, ULong.MAX_VALUE, video)
        val decoded = FrameCodec.decodePrefix(frame.encode()) as FrameDecodeResult.Complete

        assertTrue(decoded.frame.flags.contains(FrameFlags.Keyframe))
        assertEquals(ULong.MAX_VALUE, decoded.frame.sequence)
        assertEquals(video, decoded.frame.decodePayload())
    }
}
