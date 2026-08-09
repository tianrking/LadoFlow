package dev.ladoflow.display.session

import android.view.Surface
import dev.ladoflow.display.input.InputDelivery
import dev.ladoflow.display.media.VideoDecoder
import dev.ladoflow.display.media.VideoDecoderEvent
import dev.ladoflow.display.media.VideoDecoderMetrics
import dev.ladoflow.display.media.VideoDecoderState
import dev.ladoflow.display.media.coordinatedHostDisplayModes
import dev.ladoflow.display.protocol.ButtonState
import dev.ladoflow.display.protocol.CapabilitiesPayload
import dev.ladoflow.display.protocol.CodecCapabilities
import dev.ladoflow.display.protocol.CodecProfile
import dev.ladoflow.display.protocol.DisplayConfigPayload
import dev.ladoflow.display.protocol.EndpointRole
import dev.ladoflow.display.protocol.FeatureFlags
import dev.ladoflow.display.protocol.FrameFlags
import dev.ladoflow.display.protocol.HelloPayload
import dev.ladoflow.display.protocol.InputCapabilities
import dev.ladoflow.display.protocol.InputPayload
import dev.ladoflow.display.protocol.KeyInput
import dev.ladoflow.display.protocol.KeyModifiers
import dev.ladoflow.display.protocol.LdflFrame
import dev.ladoflow.display.protocol.MessageType
import dev.ladoflow.display.protocol.PingPayload
import dev.ladoflow.display.protocol.PointerButton
import dev.ladoflow.display.protocol.PointerButtonInput
import dev.ladoflow.display.protocol.PongPayload
import dev.ladoflow.display.protocol.RemoteErrorCode
import dev.ladoflow.display.protocol.RemoteErrorPayload
import dev.ladoflow.display.protocol.TelemetryPayload
import dev.ladoflow.display.protocol.VideoCodec
import dev.ladoflow.display.protocol.VideoFrameMetadata
import dev.ladoflow.display.protocol.VideoFramePayload
import dev.ladoflow.display.transport.usb.LdflDisplayTransport
import dev.ladoflow.display.transport.usb.UsbAccessoryIdentity
import dev.ladoflow.display.transport.usb.UsbTransportState
import java.util.concurrent.CopyOnWriteArrayList
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.SharedFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asSharedFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.test.runTest
import kotlinx.coroutines.withTimeout
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

@OptIn(ExperimentalCoroutinesApi::class)
class AndroidDisplaySessionTest {
    @Test
    fun repliesWithExactDisplayHelloAndCapabilitiesStartingAtSequenceZero() = runTest {
        val harness = Harness(backgroundScope)
        harness.connect()

        val helloFrame = harness.transport.nextSent()
        val capabilitiesFrame = harness.transport.nextSent()
        val hello = helloFrame.decodePayload() as HelloPayload

        assertEquals(0uL, helloFrame.sequence)
        assertEquals(MessageType.Hello, helloFrame.messageType)
        assertEquals(EndpointRole.Display, hello.role)
        assertEquals(1, hello.minProtocol)
        assertEquals(1, hello.maxProtocol)
        assertEquals("LadoFlow Android", hello.implementationName)
        assertTrue(hello.nonce.contentEquals(ByteArray(16) { 0x44 }))
        assertEquals(1uL, capabilitiesFrame.sequence)
        assertEquals(harness.localCapabilities, capabilitiesFrame.decodePayload())
        assertTrue(harness.localCapabilities.input.contains(InputCapabilities.Keyboard))
        assertEquals(FeatureFlags.None, harness.localCapabilities.features)
    }

    @Test
    fun standardFallbackModesAreAcceptedExactlyWithoutIndependentDimensionClamping() = runTest {
        val maximum = capabilities(2_732, 2_048)
        val harness = Harness(backgroundScope, maximum)
        harness.connectAndNegotiate()

        coordinatedHostDisplayModes.forEachIndexed { index, mode ->
            val configuration = DisplayConfigPayload(
                width = mode.width,
                height = mode.height,
                refreshMillihz = 60_000u,
                bitrateKbps = 12_000u,
                codec = VideoCodec.H264,
                profile = CodecProfile.H264Main,
            )
            harness.transport.framesMutable.emit(
                LdflFrame.fromPayload(FrameFlags.None, (index + 2).toULong(), configuration),
            )
            eventually { harness.decoder.configurations.size == index + 1 }
        }

        assertEquals(
            coordinatedHostDisplayModes.map { it.width to it.height },
            harness.decoder.configurations.map { it.width to it.height },
        )
    }

    @Test
    fun detachResetsActiveSessionAndExposesDeviceDisconnectedState() = runTest {
        val harness = Harness(backgroundScope)
        harness.connectAndNegotiate()
        harness.transport.framesMutable.emit(configFrame())
        eventually { harness.session.state.value is AndroidDisplaySessionState.Configured }

        harness.transport.mutableState.value = UsbTransportState.Detached(accessoryIdentity())

        eventually {
            harness.session.state.value is AndroidDisplaySessionState.DeviceDisconnected
        }
        val state = harness.session.state.value as AndroidDisplaySessionState.DeviceDisconnected
        assertEquals("Test PC", state.accessoryName)
        assertTrue(harness.decoder.resetReasons.contains("USB accessory detached"))
        assertEquals(AndroidDisplaySessionMetrics(), harness.session.metrics.value)
    }

    @Test
    fun reconnectStartsFreshHandshakeAndAcceptsFreshHostSequencesInSameProcess() = runTest {
        val harness = Harness(backgroundScope)
        harness.connectAndNegotiate()
        harness.transport.framesMutable.emit(configFrame())
        eventually { harness.session.state.value is AndroidDisplaySessionState.Configured }

        harness.transport.mutableState.value = UsbTransportState.Recovering(
            accessory = accessoryIdentity(),
            attempt = 1,
            delayMillis = 250,
            reason = "simulated read failure",
        )
        eventually { harness.session.state.value is AndroidDisplaySessionState.Recovering }
        assertTrue(harness.decoder.resetReasons.contains("USB link recovering"))

        harness.transport.mutableState.value = UsbTransportState.Connected(accessoryIdentity())
        val hello = harness.transport.nextSent()
        val capabilities = harness.transport.nextSent()
        assertEquals(listOf(0uL, 1uL), listOf(hello.sequence, capabilities.sequence))

        harness.transport.framesMutable.emit(hostHelloFrame(sequence = 0u))
        harness.transport.framesMutable.emit(hostCapabilitiesFrame(sequence = 1u))
        harness.transport.framesMutable.emit(configFrame())
        eventually { harness.decoder.configurations.size == 2 }
        harness.decoder.mutableState.value = VideoDecoderState.AwaitingKeyframe(displayConfig())

        eventually { harness.session.state.value is AndroidDisplaySessionState.Connected }
        assertEquals(2, harness.decoder.configurations.size)
    }

    @Test
    fun validHandshakeConfigAndSurfaceGateMediaBeforeDisplaying() = runTest {
        val harness = Harness(backgroundScope)
        harness.connectAndNegotiate()
        harness.transport.framesMutable.emit(configFrame())

        eventually { harness.session.state.value is AndroidDisplaySessionState.Configured }
        assertEquals(listOf(displayConfig()), harness.decoder.configurations)

        val media = videoFrame(sequence = 3u, keyframe = true)
        harness.transport.framesMutable.emit(media)
        assertTrue(harness.decoder.submitted.isEmpty())

        harness.decoder.mutableState.value = VideoDecoderState.AwaitingKeyframe(displayConfig())
        eventually { harness.decoder.submitted.size == 1 }
        assertEquals(media, harness.decoder.submitted.single())
        assertTrue(harness.session.state.value is AndroidDisplaySessionState.Connected)

        harness.decoder.mutableMetrics.value = VideoDecoderMetrics(
            outputsReleasedToSurface = 1,
            lastReleasedFrameId = 7u,
            lastDecodeDurationMicros = 2_500u,
        )
        eventually { harness.session.state.value is AndroidDisplaySessionState.Displaying }
        assertEquals(1, harness.session.metrics.value.outputsReleasedToSurface)
    }

    @Test
    fun mediaBeforeConfigFailsClosedWithConfigurationError() = runTest {
        val harness = Harness(backgroundScope)
        harness.connectAndNegotiate()

        harness.transport.framesMutable.emit(videoFrame(sequence = 2u, keyframe = true))
        eventually { harness.session.state.value is AndroidDisplaySessionState.Failed }
        assertEquals(
            DisplaySessionFailureKind.Protocol,
            (harness.session.state.value as AndroidDisplaySessionState.Failed).kind,
        )
        val error = harness.transport.nextSent().decodePayload() as RemoteErrorPayload

        assertEquals(RemoteErrorCode.ConfigurationRejected, error.code)
        assertTrue(harness.decoder.submitted.isEmpty())
    }

    @Test
    fun duplicateHelloAndDuplicateCapabilitiesEachFailClosed() = runTest {
        val duplicateHello = Harness(backgroundScope)
        duplicateHello.connectAndNegotiate()
        duplicateHello.transport.framesMutable.emit(hostHelloFrame(sequence = 2u))
        eventually { duplicateHello.session.state.value is AndroidDisplaySessionState.Failed }
        assertEquals(
            RemoteErrorCode.ProtocolViolation,
            (duplicateHello.transport.nextSent().decodePayload() as RemoteErrorPayload).code,
        )

        val duplicateCapabilities = Harness(backgroundScope)
        duplicateCapabilities.connectAndNegotiate()
        duplicateCapabilities.transport.framesMutable.emit(hostCapabilitiesFrame(sequence = 2u))
        eventually { duplicateCapabilities.session.state.value is AndroidDisplaySessionState.Failed }
        assertEquals(
            RemoteErrorCode.ProtocolViolation,
            (duplicateCapabilities.transport.nextSent().decodePayload() as RemoteErrorPayload).code,
        )
    }

    @Test
    fun pongInputAndTelemetryShareOneMonotonicControlSequence() = runTest {
        val harness = Harness(backgroundScope)
        harness.connectAndNegotiate()
        harness.transport.framesMutable.emit(configFrame())
        eventually { harness.session.state.value is AndroidDisplaySessionState.Configured }
        harness.decoder.mutableState.value = VideoDecoderState.AwaitingKeyframe(displayConfig())
        eventually { harness.session.state.value is AndroidDisplaySessionState.Connected }

        harness.transport.framesMutable.emit(
            LdflFrame.fromPayload(FrameFlags.None, 3u, PingPayload(0x99u, 100u)),
        )
        val pongFrame = harness.transport.nextSent()
        val pong = pongFrame.decodePayload() as PongPayload
        assertEquals(0x99uL, pong.token)

        harness.session.submitInputPayload(
            InputPayload(
                20_000u,
                PointerButtonInput(PointerButton.Primary, ButtonState.Pressed),
            ),
            InputDelivery.Critical,
        )
        val inputFrame = harness.transport.nextSent()
        assertTrue(harness.session.sendTelemetryNow())
        val telemetryFrame = harness.transport.nextSent()

        assertEquals(listOf(2uL, 3uL, 4uL), listOf(pongFrame, inputFrame, telemetryFrame).map { it.sequence })
        assertEquals(MessageType.Pong, pongFrame.messageType)
        assertEquals(MessageType.Input, inputFrame.messageType)
        assertTrue(telemetryFrame.decodePayload() is TelemetryPayload)
    }

    @Test
    fun sendsKeyboardInputWhenBothEndpointsAdvertiseIt() = runTest {
        val harness = Harness(backgroundScope)
        harness.connectAndNegotiate()
        harness.transport.framesMutable.emit(configFrame())
        eventually { harness.session.state.value is AndroidDisplaySessionState.Configured }
        harness.decoder.mutableState.value = VideoDecoderState.AwaitingKeyframe(displayConfig())
        eventually { harness.session.state.value is AndroidDisplaySessionState.Connected }
        val keyPayload = InputPayload(
            timestampMicros = 20_000u,
            event = KeyInput(
                usage = 0x04,
                state = ButtonState.Pressed,
                modifiers = KeyModifiers.Shift or KeyModifiers.Control,
            ),
        )

        harness.session.submitInputPayload(keyPayload, InputDelivery.Critical)
        val inputFrame = harness.transport.nextSent()

        assertEquals(2uL, inputFrame.sequence)
        assertEquals(MessageType.Input, inputFrame.messageType)
        assertEquals(keyPayload, inputFrame.decodePayload())
    }

    @Test
    fun dropsKeyboardInputWhenHostDidNotAdvertiseIt() = runTest {
        val harness = Harness(backgroundScope)
        harness.connectAndNegotiate(
            hostInput = InputCapabilities.Pointer or InputCapabilities.Touch,
        )
        harness.transport.framesMutable.emit(configFrame())
        eventually { harness.session.state.value is AndroidDisplaySessionState.Configured }
        harness.decoder.mutableState.value = VideoDecoderState.AwaitingKeyframe(displayConfig())
        eventually { harness.session.state.value is AndroidDisplaySessionState.Connected }
        val sentBeforeInput = harness.transport.sent.size

        harness.session.submitInputPayload(
            InputPayload(
                timestampMicros = 20_000u,
                event = KeyInput(0x04, ButtonState.Pressed, KeyModifiers.None),
            ),
            InputDelivery.Critical,
        )

        eventually { harness.session.metrics.value.droppedInputEvents == 1L }
        assertEquals(sentBeforeInput, harness.transport.sent.size)
    }

    @Test
    fun rejectsDisplayConfigAboveAdvertisedLimits() = runTest {
        val harness = Harness(backgroundScope)
        harness.connectAndNegotiate()
        val oversized = displayConfig().copy(width = 1_921)

        harness.transport.framesMutable.emit(
            LdflFrame.fromPayload(FrameFlags.None, 2u, oversized),
        )
        eventually { harness.session.state.value is AndroidDisplaySessionState.Failed }

        val error = harness.transport.nextSent().decodePayload() as RemoteErrorPayload
        assertEquals(RemoteErrorCode.ConfigurationRejected, error.code)
        assertTrue(harness.decoder.configurations.isEmpty())
    }

    @Test
    fun duplicateSequenceAcrossMediaAndControlFailsClosed() = runTest {
        val harness = Harness(backgroundScope)
        harness.connectAndNegotiate()
        harness.transport.framesMutable.emit(configFrame())
        eventually { harness.session.state.value is AndroidDisplaySessionState.Configured }

        harness.transport.framesMutable.emit(videoFrame(sequence = 3u, keyframe = true))
        harness.transport.framesMutable.emit(
            LdflFrame.fromPayload(FrameFlags.None, 3u, PingPayload(1u, 2u)),
        )
        eventually { harness.session.state.value is AndroidDisplaySessionState.Failed }

        val error = harness.transport.nextSent().decodePayload() as RemoteErrorPayload
        assertEquals(RemoteErrorCode.ProtocolViolation, error.code)
        assertTrue(error.diagnostic.contains("duplicate or stale", ignoreCase = true))
    }

    @Test
    fun rejectsInitialDisplayConfigThatIsNotHostSequenceTwo() = runTest {
        val harness = Harness(backgroundScope)
        harness.connectAndNegotiate()

        harness.transport.framesMutable.emit(
            LdflFrame.fromPayload(FrameFlags.None, 4u, displayConfig()),
        )
        eventually { harness.session.state.value is AndroidDisplaySessionState.Failed }

        val error = harness.transport.nextSent().decodePayload() as RemoteErrorPayload
        assertEquals(RemoteErrorCode.ConfigurationRejected, error.code)
        assertTrue(harness.decoder.configurations.isEmpty())
    }

    @Test
    fun rejectsH264ProfileOtherThanMain() = runTest {
        val harness = Harness(backgroundScope)
        harness.connectAndNegotiate()
        val highProfile = displayConfig().copy(profile = CodecProfile.H264High)

        harness.transport.framesMutable.emit(
            LdflFrame.fromPayload(FrameFlags.None, 2u, highProfile),
        )
        eventually { harness.session.state.value is AndroidDisplaySessionState.Failed }

        val error = harness.transport.nextSent().decodePayload() as RemoteErrorPayload
        assertEquals(RemoteErrorCode.ConfigurationRejected, error.code)
        assertTrue(harness.decoder.configurations.isEmpty())
    }

    @Test
    fun telemetryReportsLastSurfaceReleasedHostFrameAndSessionQueueCounters() = runTest {
        val harness = Harness(backgroundScope)
        harness.connectAndNegotiate()
        harness.transport.framesMutable.emit(configFrame())
        eventually { harness.session.state.value is AndroidDisplaySessionState.Configured }
        harness.decoder.mutableState.value = VideoDecoderState.AwaitingKeyframe(displayConfig())
        eventually { harness.session.state.value is AndroidDisplaySessionState.Connected }

        harness.decoder.mutableMetrics.value = VideoDecoderMetrics(
            outputsReleasedToSurface = 1,
            droppedFrames = 4,
            queueDepth = 2,
            lastReleasedFrameId = 0x1234u,
            lastDecodeDurationMicros = 3_250u,
        )
        eventually {
            harness.session.metrics.value.outputsReleasedToSurface == 1L &&
                harness.session.metrics.value.droppedVideoFrames == 4L
        }

        assertTrue(harness.session.sendTelemetryNow())
        val telemetry = harness.transport.nextSent().decodePayload() as TelemetryPayload
        assertEquals(0x1234uL, telemetry.frameId)
        assertEquals(4u, telemetry.droppedFrames)
        assertEquals(2, telemetry.queueDepth)
        assertEquals(3_250u, telemetry.timings.decodeMicros)
        assertEquals(0u, telemetry.timings.presentationMicros)
    }

    @Test
    fun rebuiltSurfaceNeedsANewerCorrelatedOutputBeforeDisplayingAgain() = runTest {
        val harness = Harness(backgroundScope)
        harness.connectAndNegotiate()
        harness.transport.framesMutable.emit(configFrame())
        eventually { harness.session.state.value is AndroidDisplaySessionState.Configured }
        harness.decoder.mutableState.value = VideoDecoderState.AwaitingKeyframe(displayConfig())
        eventually { harness.session.state.value is AndroidDisplaySessionState.Connected }
        harness.decoder.mutableMetrics.value = VideoDecoderMetrics(
            outputsReleasedToSurface = 1,
            lastReleasedFrameId = 10u,
            lastDecodeDurationMicros = 2_000u,
        )
        eventually { harness.session.state.value is AndroidDisplaySessionState.Displaying }

        harness.decoder.mutableState.value = VideoDecoderState.AwaitingSurface(displayConfig())
        eventually { harness.session.state.value is AndroidDisplaySessionState.Configured }
        harness.decoder.mutableState.value = VideoDecoderState.AwaitingKeyframe(displayConfig())
        eventually { harness.session.state.value is AndroidDisplaySessionState.Connected }
        harness.decoder.mutableMetrics.value = harness.decoder.mutableMetrics.value.copy(
            queueDepth = 1,
        )
        eventually { harness.session.metrics.value.queueDepth == 1 }
        assertTrue(harness.session.state.value is AndroidDisplaySessionState.Connected)

        harness.decoder.mutableMetrics.value = VideoDecoderMetrics(
            outputsReleasedToSurface = 2,
            lastReleasedFrameId = 11u,
            lastDecodeDurationMicros = 2_500u,
        )
        eventually { harness.session.state.value is AndroidDisplaySessionState.Displaying }
        assertTrue(harness.session.sendTelemetryNow())
        val telemetry = harness.transport.nextSent().decodePayload() as TelemetryPayload
        assertEquals(11uL, telemetry.frameId)
    }

    private class Harness(
        scope: kotlinx.coroutines.CoroutineScope,
        val localCapabilities: CapabilitiesPayload = capabilities(1_920, 1_080),
    ) {
        val transport = FakeTransport()
        val decoder = FakeDecoder()
        val session = AndroidDisplaySession(
            transport = transport,
            decoder = decoder,
            localCapabilities = localCapabilities,
            parentScope = scope,
            nonceSource = { ByteArray(16) { 0x44 } },
            monotonicMicros = generateSequence(1_000uL) { it + 100u }.iterator()::next,
            periodicReportsEnabled = false,
        ).also { it.start() }

        suspend fun connect() {
            transport.mutableState.value = UsbTransportState.Connected(accessoryIdentity())
        }

        suspend fun connectAndNegotiate(
            hostInput: InputCapabilities = TEST_INPUT_CAPABILITIES,
        ) {
            connect()
            transport.nextSent()
            transport.nextSent()
            transport.framesMutable.emit(hostHelloFrame())
            transport.framesMutable.emit(hostCapabilitiesFrame(input = hostInput))
            eventually { session.state.value is AndroidDisplaySessionState.Ready }
        }
    }

    private class FakeTransport : LdflDisplayTransport {
        val mutableState = MutableStateFlow<UsbTransportState>(UsbTransportState.Stopped)
        val framesMutable = MutableSharedFlow<LdflFrame>(extraBufferCapacity = 16)
        private val sentChannel = Channel<LdflFrame>(Channel.UNLIMITED)
        val sent = CopyOnWriteArrayList<LdflFrame>()
        override val state: StateFlow<UsbTransportState> = mutableState.asStateFlow()
        override val frames: Flow<LdflFrame> = framesMutable

        override suspend fun sendControl(frame: LdflFrame): Boolean {
            sent += frame
            sentChannel.send(frame)
            return true
        }

        override fun trySendControl(frame: LdflFrame): Boolean {
            sent += frame
            return sentChannel.trySend(frame).isSuccess
        }

        override fun retry() = Unit

        override fun disconnect() {
            mutableState.value = UsbTransportState.Stopped
        }

        suspend fun nextSent(): LdflFrame = withTimeout(5_000) { sentChannel.receive() }
    }

    private class FakeDecoder : VideoDecoder {
        val mutableState = MutableStateFlow<VideoDecoderState>(VideoDecoderState.Idle)
        val mutableMetrics = MutableStateFlow(VideoDecoderMetrics())
        val eventsMutable = MutableSharedFlow<VideoDecoderEvent>(extraBufferCapacity = 16)
        val configurations = mutableListOf<DisplayConfigPayload>()
        val submitted = mutableListOf<LdflFrame>()
        val resetReasons = mutableListOf<String>()
        override val state: StateFlow<VideoDecoderState> = mutableState.asStateFlow()
        override val events: SharedFlow<VideoDecoderEvent> = eventsMutable.asSharedFlow()
        override val metrics: StateFlow<VideoDecoderMetrics> = mutableMetrics.asStateFlow()

        override fun setOutputSurface(surface: Surface?) = Unit

        override fun applyConfiguration(configuration: DisplayConfigPayload) {
            configurations += configuration
            mutableState.value = VideoDecoderState.AwaitingSurface(configuration)
        }

        override fun submit(frame: LdflFrame): Boolean = submitted.add(frame)

        override fun reset(reason: String) {
            resetReasons += reason
            mutableState.value = VideoDecoderState.Idle
        }

        override fun close() {
            mutableState.value = VideoDecoderState.Closed
        }
    }

    companion object {
        private fun capabilities(
            width: Int,
            height: Int,
            input: InputCapabilities = TEST_INPUT_CAPABILITIES,
        ) = CapabilitiesPayload(
            maxWidth = width,
            maxHeight = height,
            maxRefreshMillihz = 60_000u,
            maxBitrateKbps = 20_000u,
            codecs = CodecCapabilities.H264,
            input = input,
            features = FeatureFlags.None,
        )

        private fun hostHelloFrame(sequence: ULong = 0u) = LdflFrame.fromPayload(
            FrameFlags.None,
            sequence,
            HelloPayload(1, 1, EndpointRole.Host, ByteArray(16) { 0x11 }, "Windows Host"),
        )

        private fun hostCapabilitiesFrame(
            sequence: ULong = 1u,
            input: InputCapabilities = TEST_INPUT_CAPABILITIES,
        ) = LdflFrame.fromPayload(
            FrameFlags.None,
            sequence,
            capabilities(3_840, 2_160, input),
        )

        private fun displayConfig() = DisplayConfigPayload(
            width = 1_920,
            height = 1_080,
            refreshMillihz = 60_000u,
            bitrateKbps = 12_000u,
            codec = VideoCodec.H264,
            profile = CodecProfile.H264Main,
        )

        private fun configFrame() = LdflFrame.fromPayload(FrameFlags.None, 2u, displayConfig())

        private fun videoFrame(
            sequence: ULong,
            keyframe: Boolean,
        ) = LdflFrame.fromPayload(
            if (keyframe) FrameFlags.Keyframe else FrameFlags.None,
            sequence,
            VideoFramePayload(
                VideoFrameMetadata(sequence, 10u, 20u, 16_667u),
                byteArrayOf(0, 0, 1, 0x65, 1, 2),
            ),
        )

        private fun accessoryIdentity() = UsbAccessoryIdentity(
            manufacturer = "LadoFlow",
            model = "LadoFlow Host",
            description = "Test PC",
            version = "test",
            serial = null,
        )

        private suspend fun eventually(predicate: () -> Boolean) {
            withTimeout(5_000) {
                while (!predicate()) kotlinx.coroutines.yield()
            }
        }
    }
}

private val TEST_INPUT_CAPABILITIES =
    InputCapabilities.Pointer or InputCapabilities.Touch or InputCapabilities.Keyboard
