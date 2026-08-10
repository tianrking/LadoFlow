package dev.ladoflow.display.protocol

import org.junit.Assert.assertEquals
import org.junit.Test

class TypedPayloadCodecTest {
    @Test
    fun `all version-one payload families round trip through dispatch`() {
        val payloads = listOf<LdflPayload>(
            HelloPayload(1, 1, EndpointRole.Display, ByteArray(16) { it.toByte() }, "Android"),
            CapabilitiesPayload(
                maxWidth = 2560,
                maxHeight = 1600,
                maxRefreshMillihz = 120_000u,
                maxBitrateKbps = 40_000u,
                codecs = CodecCapabilities.H264 or CodecCapabilities.Hevc,
                input = InputCapabilities.Pointer or InputCapabilities.Touch or InputCapabilities.Keyboard,
                features = FeatureFlags.DynamicRotation or FeatureFlags.RemoteCursor,
            ),
            DisplayConfigPayload(1920, 1080, 60_000u, 20_000u, VideoCodec.H264, CodecProfile.H264High),
            VideoFramePayload(VideoFrameMetadata(9uL, 10uL, 11uL, 16_667u), byteArrayOf(1, 2, 3)),
            InputPayload(12uL, TouchInput(15, TouchPhase.Move, 100, 200, 32_768)),
            TelemetryPayload(
                13uL,
                9uL,
                StageTimings(1u, 2u, 3u, 4u, 5u),
                3,
                4u,
                5u,
                6u,
                ThermalState.Nominal,
            ),
            PingPayload(14uL, 15uL),
            PongPayload(14uL, 15uL, 16uL, 17uL),
            RemoteErrorPayload(RemoteErrorCode.DecoderFailure, true, "needs keyframe"),
        )

        payloads.forEach { expected ->
            assertEquals(expected, PayloadCodec.decode(expected.messageType, expected.encode()))
        }
    }

    @Test
    fun `every input event variant round trips`() {
        val events = listOf<InputEventBody>(
            PointerMoveInput(1920, 1080),
            PointerButtonInput(PointerButton.Secondary, ButtonState.Pressed),
            WheelInput((-120).toShort(), 240.toShort()),
            KeyInput(4, ButtonState.Released, KeyModifiers.Shift or KeyModifiers.Control),
            TouchInput(15, TouchPhase.Move, 123, 456, 32_768),
            FocusInput(true),
        )

        events.forEachIndexed { index, event ->
            val expected = InputPayload((100 + index).toULong(), event)
            assertEquals(expected, InputPayload.decode(expected.encode()))
        }
    }

    @Test
    fun `typed decoder rejects malformed values and noncanonical lengths`() {
        assertViolation(ProtocolViolation.InvalidPayload) {
            HelloPayload.decode(ByteArray(21))
        }
        assertViolation(ProtocolViolation.InvalidUtf8) {
            val bytes = HelloPayload(1, 1, EndpointRole.Display, ByteArray(16), "x").encode()
            bytes[22] = 0xff.toByte()
            HelloPayload.decode(bytes)
        }
        assertViolation(ProtocolViolation.InvalidPayload) {
            val bytes = validCapabilities().encode()
            bytes[12] = 0x80.toByte()
            CapabilitiesPayload.decode(bytes)
        }
        assertViolation(ProtocolViolation.InvalidPayload) {
            val bytes = DisplayConfigPayload(
                1920,
                1080,
                60_000u,
                20_000u,
                VideoCodec.H264,
                CodecProfile.H264High,
            ).encode()
            bytes[13] = CodecProfile.Av1Main.wireValue.toByte()
            DisplayConfigPayload.decode(bytes)
        }
        assertViolation(ProtocolViolation.InvalidPayload) {
            VideoFramePayload.decode(ByteArray(VIDEO_FRAME_METADATA_BYTES))
        }
        assertViolation(ProtocolViolation.InvalidPayload) {
            val focus = InputPayload(1uL, FocusInput(true)).encode()
            focus[9] = 2
            InputPayload.decode(focus)
        }
        assertViolation(ProtocolViolation.InvalidPayload) {
            val telemetry = validTelemetry().encode()
            telemetry[50] = 99
            TelemetryPayload.decode(telemetry)
        }
        assertViolation(ProtocolViolation.InvalidPayload) {
            PongPayload(1uL, 2uL, 4uL, 3uL)
        }
        assertViolation(ProtocolViolation.InvalidUtf8) {
            val error = RemoteErrorPayload(RemoteErrorCode.Internal, false, "x").encode()
            error[5] = 0xff.toByte()
            RemoteErrorPayload.decode(error)
        }
    }

    @Test
    fun `constructors enforce documented resource bounds`() {
        assertViolation(ProtocolViolation.InvalidPayload) {
            CapabilitiesPayload(
                0,
                1080,
                60_000u,
                20_000u,
                CodecCapabilities.H264,
                InputCapabilities.None,
                FeatureFlags.None,
            )
        }
        assertViolation(ProtocolViolation.PayloadTooLarge) {
            VideoFramePayload(
                VideoFrameMetadata(1uL, 2uL, 3uL, 1u),
                ByteArray(MAX_ENCODED_VIDEO_BYTES + 1),
            )
        }
        assertViolation(ProtocolViolation.InvalidPayload) {
            KeyInput(0, ButtonState.Pressed, KeyModifiers.None)
        }
        assertViolation(ProtocolViolation.InvalidPayload) {
            TouchInput(MAX_TOUCH_CONTACTS, TouchPhase.Begin, 0, 0, 0)
        }
        assertViolation(ProtocolViolation.InvalidPayload) {
            StageTimings(MAX_STAGE_DURATION_MICROS + 1u, 0u, 0u, 0u, 0u)
        }
        assertViolation(ProtocolViolation.InvalidPayload) {
            validTelemetry().copy(queueDepth = MAX_TELEMETRY_QUEUE_DEPTH + 1)
        }
        assertViolation(ProtocolViolation.InvalidPayload) {
            RemoteErrorPayload(RemoteErrorCode.Internal, false, "x".repeat(1_025))
        }
    }

    @Test
    fun `typed frame decode rejects the wrong message family before payload parsing`() {
        val pingFrame = LdflFrame(MessageType.Ping, FrameFlags.None, 1uL, ByteArray(0))

        assertViolation(ProtocolViolation.UnexpectedMessageType) {
            PayloadCodec.decodeAs<HelloPayload>(pingFrame)
        }
    }

    private fun validCapabilities() = CapabilitiesPayload(
        1920,
        1080,
        60_000u,
        20_000u,
        CodecCapabilities.H264,
        InputCapabilities.None,
        FeatureFlags.None,
    )

    private fun validTelemetry() = TelemetryPayload(
        1uL,
        2uL,
        StageTimings(1u, 2u, 3u, 4u, 5u),
        3,
        4u,
        5u,
        6u,
        ThermalState.Nominal,
    )
}
