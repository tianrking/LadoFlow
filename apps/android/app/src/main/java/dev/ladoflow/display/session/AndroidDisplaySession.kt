package dev.ladoflow.display.session

import android.os.SystemClock
import dev.ladoflow.display.input.AndroidInputController
import dev.ladoflow.display.input.AndroidInputEmission
import dev.ladoflow.display.input.DisplayRotation
import dev.ladoflow.display.input.InputDelivery
import dev.ladoflow.display.input.RemoteViewport
import dev.ladoflow.display.media.DecoderSurfaceController
import dev.ladoflow.display.media.VideoDecoder
import dev.ladoflow.display.media.VideoDecoderEvent
import dev.ladoflow.display.media.VideoDecoderMetrics
import dev.ladoflow.display.media.VideoDecoderState
import dev.ladoflow.display.protocol.CapabilitiesPayload
import dev.ladoflow.display.protocol.CodecCapabilities
import dev.ladoflow.display.protocol.CodecProfile
import dev.ladoflow.display.protocol.DisplayConfigPayload
import dev.ladoflow.display.protocol.EndpointRole
import dev.ladoflow.display.protocol.FeatureFlags
import dev.ladoflow.display.protocol.HelloPayload
import dev.ladoflow.display.protocol.InputCapabilities
import dev.ladoflow.display.protocol.FocusInput
import dev.ladoflow.display.protocol.InputPayload
import dev.ladoflow.display.protocol.KeyInput
import dev.ladoflow.display.protocol.LDFL_PROTOCOL_VERSION
import dev.ladoflow.display.protocol.LdflFrame
import dev.ladoflow.display.protocol.MessageType
import dev.ladoflow.display.protocol.MAX_TELEMETRY_QUEUE_DEPTH
import dev.ladoflow.display.protocol.MonotonicSequenceValidator
import dev.ladoflow.display.protocol.PingPayload
import dev.ladoflow.display.protocol.PointerButtonInput
import dev.ladoflow.display.protocol.PointerMoveInput
import dev.ladoflow.display.protocol.PongPayload
import dev.ladoflow.display.protocol.RemoteErrorCode
import dev.ladoflow.display.protocol.RemoteErrorPayload
import dev.ladoflow.display.protocol.StageTimings
import dev.ladoflow.display.protocol.TelemetryPayload
import dev.ladoflow.display.protocol.ThermalState
import dev.ladoflow.display.protocol.TouchInput
import dev.ladoflow.display.protocol.VideoCodec
import dev.ladoflow.display.protocol.WheelInput
import dev.ladoflow.display.transport.usb.LdflDisplayTransport
import dev.ladoflow.display.transport.usb.UsbTransportState
import java.io.Closeable
import java.security.SecureRandom
import java.util.concurrent.atomic.AtomicBoolean
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineName
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch

sealed interface AndroidDisplaySessionState {
    data object Stopped : AndroidDisplaySessionState

    data object WaitingForAccessory : AndroidDisplaySessionState

    data class WaitingForPermission(val accessoryName: String) : AndroidDisplaySessionState

    data class Handshaking(val accessoryName: String) : AndroidDisplaySessionState

    data class Ready(val hostName: String) : AndroidDisplaySessionState

    data class Configured(
        val hostName: String,
        val configuration: DisplayConfigPayload,
    ) : AndroidDisplaySessionState

    data class Connected(
        val hostName: String,
        val configuration: DisplayConfigPayload,
    ) : AndroidDisplaySessionState

    data class DeviceDisconnected(val accessoryName: String) : AndroidDisplaySessionState

    data class Displaying(
        val hostName: String,
        val configuration: DisplayConfigPayload,
    ) : AndroidDisplaySessionState

    data class Recovering(
        val attempt: Int,
        val reason: String,
    ) : AndroidDisplaySessionState

    data class Failed(
        val reason: String,
        val retryable: Boolean,
        val kind: DisplaySessionFailureKind = DisplaySessionFailureKind.Local,
    ) : AndroidDisplaySessionState

    data class Unsupported(val reason: String) : AndroidDisplaySessionState
}

enum class DisplaySessionFailureKind {
    Transport,
    Protocol,
    Decoder,
    Local,
}

data class AndroidDisplaySessionMetrics(
    val outputsReleasedToSurface: Long = 0,
    val droppedVideoFrames: Long = 0,
    val droppedInputEvents: Long = 0,
    val queueDepth: Int = 0,
    val latestDecodeDurationMicros: UInt? = null,
)

/** Owns LDFL v1 negotiation and composes transport, decoder, Surface, and reverse input. */
class AndroidDisplaySession(
    private val transport: LdflDisplayTransport,
    val decoder: VideoDecoder,
    val localCapabilities: CapabilitiesPayload,
    parentScope: CoroutineScope,
    private val implementationName: String = "LadoFlow Android",
    private val nonceSource: () -> ByteArray = ::secureNonce,
    private val monotonicMicros: () -> ULong = ::elapsedRealtimeMicros,
    private val periodicReportsEnabled: Boolean = true,
) : Closeable {
    private val sessionJob = SupervisorJob(parentScope.coroutineContext[Job])
    private val scope = CoroutineScope(
        parentScope.coroutineContext + sessionJob + CoroutineName("LadoFlow display session"),
    )
    private val started = AtomicBoolean(false)
    private val closed = AtomicBoolean(false)
    private val mutableState = MutableStateFlow<AndroidDisplaySessionState>(
        AndroidDisplaySessionState.Stopped,
    )
    private val mutableMetrics = MutableStateFlow(AndroidDisplaySessionMetrics())

    private var outbound: SessionOutboundMux? = null
    private var accessoryName = "LadoFlow Host"
    private var remoteHello: HelloPayload? = null
    private var remoteCapabilities: CapabilitiesPayload? = null
    private var negotiated = false
    private var failed = false
    private var activeConfiguration: DisplayConfigPayload? = null
    private var protocolGeneration = 0L
    private var surfaceReady = false
    private var awaitingSurfaceKeyframe = true
    private val pendingSurfaceFrames = java.util.ArrayDeque<LdflFrame>(3)
    private var localDroppedVideoFrames = 0L
    private var surfaceOutputBaseline = 0L
    private var lastReleasedFrameId = 0uL
    private var pingToken = 0uL
    private var inboundSequences = MonotonicSequenceValidator()

    val state: StateFlow<AndroidDisplaySessionState> = mutableState.asStateFlow()
    val metrics: StateFlow<AndroidDisplaySessionMetrics> = mutableMetrics.asStateFlow()
    val inputController = AndroidInputController(::submitInput, monotonicMicros)
    val surfaceController = DecoderSurfaceController(decoder)

    fun start() {
        if (!started.compareAndSet(false, true)) return
        scope.launch { transport.state.collect(::handleTransportState) }
        scope.launch { transport.frames.collect(::handleInboundFrame) }
        scope.launch { decoder.events.collect(::handleDecoderEvent) }
        scope.launch { decoder.state.collect(::handleDecoderState) }
        scope.launch { decoder.metrics.collect(::handleDecoderMetrics) }
        if (periodicReportsEnabled) {
            scope.launch { telemetryLoop() }
            scope.launch { pingLoop() }
        }
    }

    fun retry() {
        if (closed.get()) return
        transport.disconnect()
        transport.retry()
    }

    fun disconnect() {
        if (closed.get()) return
        transport.disconnect()
    }

    private suspend fun handleTransportState(transportState: UsbTransportState) {
        if (closed.get()) return
        when (transportState) {
            UsbTransportState.Stopped -> {
                resetProtocol("USB transport stopped")
                mutableState.value = AndroidDisplaySessionState.Stopped
            }

            UsbTransportState.WaitingForAccessory -> {
                resetProtocol("Waiting for USB accessory")
                mutableState.value = AndroidDisplaySessionState.WaitingForAccessory
            }

            is UsbTransportState.AwaitingPermission -> {
                resetProtocol("Waiting for Android USB permission")
                accessoryName = transportState.accessory.displayName
                mutableState.value = AndroidDisplaySessionState.WaitingForPermission(accessoryName)
            }

            is UsbTransportState.Opening -> {
                accessoryName = transportState.accessory.displayName
                mutableState.value = AndroidDisplaySessionState.Handshaking(accessoryName)
            }

            is UsbTransportState.Connected -> beginHandshake(transportState.accessory.displayName)

            is UsbTransportState.Detached -> {
                resetProtocol("USB accessory detached")
                accessoryName = transportState.accessory.displayName
                mutableState.value = AndroidDisplaySessionState.DeviceDisconnected(accessoryName)
            }

            is UsbTransportState.Recovering -> {
                resetProtocol("USB link recovering")
                mutableState.value = AndroidDisplaySessionState.Recovering(
                    attempt = transportState.attempt,
                    reason = transportState.reason,
                )
            }

            is UsbTransportState.Error -> {
                resetProtocol("USB transport error")
                mutableState.value = AndroidDisplaySessionState.Failed(
                    reason = transportState.reason,
                    retryable = transportState.retryable,
                    kind = DisplaySessionFailureKind.Transport,
                )
            }

            is UsbTransportState.Unsupported -> {
                resetProtocol("USB accessory unsupported")
                mutableState.value = AndroidDisplaySessionState.Unsupported(transportState.reason)
            }
        }
    }

    private suspend fun beginHandshake(name: String) {
        resetProtocol("Starting a new LDFL generation")
        accessoryName = name
        val generation = protocolGeneration
        outbound = SessionOutboundMux(
            parentScope = scope,
            sendFrame = { frame ->
                if (generation != protocolGeneration) false else transport.sendControl(frame)
            },
            onSendFailure = { reason ->
                if (generation == protocolGeneration && !closed.get()) {
                    failed = true
                    mutableState.value = AndroidDisplaySessionState.Failed(
                        reason,
                        retryable = true,
                        kind = DisplaySessionFailureKind.Transport,
                    )
                }
            },
        )
        mutableState.value = AndroidDisplaySessionState.Handshaking(name)
        val nonce = nonceSource()
        if (nonce.size != 16) {
            failLocal("Nonce source returned ${nonce.size} bytes instead of 16")
            return
        }
        outbound?.sendControl(
            HelloPayload(
                minProtocol = LDFL_PROTOCOL_VERSION,
                maxProtocol = LDFL_PROTOCOL_VERSION,
                role = EndpointRole.Display,
                nonce = nonce,
                implementationName = implementationName,
            ),
        )
        outbound?.sendControl(localCapabilities)
    }

    private suspend fun handleInboundFrame(frame: LdflFrame) {
        try {
            inboundSequences.observe(frame.sequence)
        } catch (exception: IllegalArgumentException) {
            rejectProtocol(
                RemoteErrorCode.ProtocolViolation,
                exception.message ?: "Duplicate or stale LDFL sender sequence",
            )
            return
        }
        if (frame.messageType == MessageType.VideoFrame) {
            handleMediaFrame(frame)
        } else {
            handleControlFrame(frame)
        }
    }

    private suspend fun handleControlFrame(frame: LdflFrame) {
        if (closed.get() || failed) return
        val payload = try {
            frame.decodePayload()
        } catch (exception: IllegalArgumentException) {
            rejectProtocol(
                RemoteErrorCode.ProtocolViolation,
                exception.message ?: "Invalid LDFL control payload",
            )
            return
        }
        when (payload) {
            is HelloPayload -> handleRemoteHello(frame, payload)
            is CapabilitiesPayload -> handleRemoteCapabilities(frame, payload)
            is DisplayConfigPayload -> handleDisplayConfiguration(frame, payload)
            is PingPayload -> ifActive("Ping") { respondToPing(payload) }
            is PongPayload -> ifActive("Pong") { }
            is TelemetryPayload -> ifActive("Telemetry") { }
            is RemoteErrorPayload -> {
                activeConfiguration = null
                negotiated = false
                failed = true
                inputController.updateViewport(null)
                decoder.reset("Host reported ${payload.code}")
                mutableState.value = AndroidDisplaySessionState.Failed(
                    reason = payload.diagnostic.ifEmpty { "Host reported ${payload.code}" },
                    retryable = payload.retryable,
                    kind = DisplaySessionFailureKind.Protocol,
                )
            }

            is InputPayload -> rejectProtocol(
                RemoteErrorCode.InputRejected,
                "Host-to-display Input frames are invalid in LDFL v1",
            )

            else -> rejectProtocol(
                RemoteErrorCode.ProtocolViolation,
                "${payload.messageType} arrived on the control stream unexpectedly",
            )
        }
    }

    private suspend fun handleRemoteHello(
        frame: LdflFrame,
        hello: HelloPayload,
    ) {
        if (frame.sequence != 0uL || remoteHello != null || remoteCapabilities != null) {
            rejectProtocol(
                RemoteErrorCode.ProtocolViolation,
                "Host Hello must be the first frame at sequence 0 and may appear once",
            )
            return
        }
        if (hello.role != EndpointRole.Host) {
            rejectProtocol(RemoteErrorCode.ProtocolViolation, "Remote Hello role must be Host")
            return
        }
        if (LDFL_PROTOCOL_VERSION !in hello.minProtocol..hello.maxProtocol) {
            rejectProtocol(RemoteErrorCode.Unsupported, "Host does not support LDFL v1")
            return
        }
        remoteHello = hello
        finishNegotiationIfReady()
    }

    private suspend fun handleRemoteCapabilities(
        frame: LdflFrame,
        capabilities: CapabilitiesPayload,
    ) {
        if (frame.sequence != 1uL || remoteHello == null || remoteCapabilities != null) {
            rejectProtocol(
                RemoteErrorCode.ProtocolViolation,
                "Host Capabilities must follow Hello at sequence 1 and may appear once",
            )
            return
        }
        remoteCapabilities = capabilities
        finishNegotiationIfReady()
    }

    private suspend fun finishNegotiationIfReady() {
        val hello = remoteHello ?: return
        val capabilities = remoteCapabilities ?: return
        if (!localCapabilities.codecs.contains(CodecCapabilities.H264) ||
            !capabilities.codecs.contains(CodecCapabilities.H264)
        ) {
            rejectProtocol(RemoteErrorCode.Unsupported, "No common H.264 codec capability")
            return
        }
        negotiated = true
        mutableState.value = AndroidDisplaySessionState.Ready(hello.implementationName)
    }

    private suspend fun handleDisplayConfiguration(
        frame: LdflFrame,
        configuration: DisplayConfigPayload,
    ) {
        val hostCapabilities = remoteCapabilities
        if (!negotiated || hostCapabilities == null || remoteHello == null) {
            rejectProtocol(
                RemoteErrorCode.ConfigurationRejected,
                "DisplayConfig arrived before Hello/Capabilities negotiation completed",
            )
            return
        }
        if (activeConfiguration == null && frame.sequence != 2uL) {
            rejectProtocol(
                RemoteErrorCode.ConfigurationRejected,
                "Initial DisplayConfig must be host sequence 2",
            )
            return
        }
        val maximumWidth = minOf(localCapabilities.maxWidth, hostCapabilities.maxWidth)
        val maximumHeight = minOf(localCapabilities.maxHeight, hostCapabilities.maxHeight)
        val maximumRefresh = minOf(
            localCapabilities.maxRefreshMillihz,
            hostCapabilities.maxRefreshMillihz,
        )
        val maximumBitrate = minOf(localCapabilities.maxBitrateKbps, hostCapabilities.maxBitrateKbps)
        val decoderRejection = if (
            configuration.codec == VideoCodec.H264 &&
            configuration.profile == CodecProfile.H264Main
        ) {
            decoder.configurationRejectionReason(configuration)
        } else {
            null
        }
        val invalidReason = when {
            configuration.codec != VideoCodec.H264 -> "Android v1 currently enables H.264 only"
            configuration.profile != CodecProfile.H264Main ->
                "Android v1 session currently accepts H.264 Main only"
            configuration.width > maximumWidth || configuration.height > maximumHeight ->
                "DisplayConfig ${configuration.width}x${configuration.height} exceeds negotiated limits"

            configuration.refreshMillihz > maximumRefresh ->
                "DisplayConfig refresh exceeds negotiated limits"

            configuration.bitrateKbps > maximumBitrate ->
                "DisplayConfig bitrate exceeds negotiated limits"

            decoderRejection != null -> decoderRejection

            else -> null
        }
        if (invalidReason != null) {
            rejectProtocol(RemoteErrorCode.ConfigurationRejected, invalidReason)
            return
        }
        if (activeConfiguration == configuration) return

        activeConfiguration = configuration
        surfaceReady = false
        surfaceOutputBaseline = decoder.metrics.value.outputsReleasedToSurface
        awaitingSurfaceKeyframe = true
        pendingSurfaceFrames.clear()
        inputController.updateViewport(
            RemoteViewport(
                codedWidth = configuration.width,
                codedHeight = configuration.height,
                rotation = DisplayRotation.Degrees0,
            ),
        )
        decoder.applyConfiguration(configuration)
        mutableState.value = AndroidDisplaySessionState.Configured(
            hostName = requireNotNull(remoteHello).implementationName,
            configuration = configuration,
        )
    }

    private suspend fun ifActive(
        frameName: String,
        action: suspend () -> Unit,
    ) {
        if (activeConfiguration == null) {
            rejectProtocol(
                RemoteErrorCode.ProtocolViolation,
                "$frameName arrived before DisplayConfig was accepted",
            )
        } else {
            action()
        }
    }

    private suspend fun respondToPing(ping: PingPayload) {
        val receiveTimestamp = monotonicMicros()
        val sendTimestamp = monotonicMicros().coerceAtLeast(receiveTimestamp)
        outbound?.sendControl(
            PongPayload(
                token = ping.token,
                clientSendTimestampMicros = ping.clientSendTimestampMicros,
                serverReceiveTimestampMicros = receiveTimestamp,
                serverSendTimestampMicros = sendTimestamp,
            ),
        )
    }

    private suspend fun handleMediaFrame(frame: LdflFrame) {
        if (closed.get() || failed) return
        if (frame.messageType != MessageType.VideoFrame) {
            rejectProtocol(RemoteErrorCode.ProtocolViolation, "Non-video frame entered media lane")
            return
        }
        if (!negotiated || activeConfiguration == null) {
            rejectProtocol(
                RemoteErrorCode.ConfigurationRejected,
                "VideoFrame arrived before an accepted DisplayConfig",
            )
            return
        }
        if (!surfaceReady) {
            enqueueUntilSurfaceReady(frame)
        } else if (!decoder.submit(frame)) {
            recordDroppedVideo()
        }
    }

    private fun submitInput(emission: AndroidInputEmission) {
        if (mutableState.value !is AndroidDisplaySessionState.Connected &&
            mutableState.value !is AndroidDisplaySessionState.Displaying
        ) {
            return
        }
        val negotiatedInputCapabilities = remoteCapabilities?.input?.let { hostInput ->
            InputCapabilities.fromBits(localCapabilities.input.bits and hostInput.bits)
        }
        if (negotiatedInputCapabilities == null ||
            !emission.payload.isSupportedBy(negotiatedInputCapabilities)
        ) {
            recordDroppedInput()
            return
        }
        val currentOutbound = outbound ?: return
        when (emission.delivery) {
            InputDelivery.Critical -> scope.launch {
                try {
                    currentOutbound.sendCriticalInput(emission.payload)
                } catch (cancelled: CancellationException) {
                    throw cancelled
                } catch (_: Exception) {
                    recordDroppedInput()
                }
            }

            InputDelivery.Coalescible -> {
                if (!currentOutbound.trySendCoalescibleInput(emission.payload)) recordDroppedInput()
            }
        }
    }

    internal fun submitInputPayload(
        payload: InputPayload,
        delivery: InputDelivery,
    ) {
        submitInput(AndroidInputEmission(payload, delivery))
    }

    private fun handleDecoderEvent(event: VideoDecoderEvent) {
        when (event) {
            is VideoDecoderEvent.FrameDropped,
            is VideoDecoderEvent.OutputReleasedToSurface,
            is VideoDecoderEvent.Failure,
            -> Unit

            VideoDecoderEvent.EndOfStream -> {
                activeConfiguration = null
                inputController.updateViewport(null)
                remoteHello?.let {
                    mutableState.value = AndroidDisplaySessionState.Ready(it.implementationName)
                }
            }

            is VideoDecoderEvent.OutputFormatChanged,
            is VideoDecoderEvent.Warning,
            -> Unit
        }
    }

    private fun handleDecoderMetrics(decoderMetrics: VideoDecoderMetrics) {
        val configuration = activeConfiguration ?: return
        val hello = remoteHello ?: return
        val correlatedFrameId = decoderMetrics.lastReleasedFrameId
        if (correlatedFrameId != null) lastReleasedFrameId = correlatedFrameId
        mutableMetrics.value = mutableMetrics.value.copy(
            outputsReleasedToSurface = decoderMetrics.outputsReleasedToSurface,
            droppedVideoFrames = localDroppedVideoFrames.saturatingAdd(
                decoderMetrics.droppedFrames,
            ),
            queueDepth = (pendingSurfaceFrames.size + decoderMetrics.queueDepth)
                .coerceAtMost(MAX_TELEMETRY_QUEUE_DEPTH),
            latestDecodeDurationMicros = decoderMetrics.lastDecodeDurationMicros,
        )
        if (
            surfaceReady &&
            correlatedFrameId != null &&
            decoderMetrics.outputsReleasedToSurface > surfaceOutputBaseline
        ) {
            mutableState.value = AndroidDisplaySessionState.Displaying(
                hello.implementationName,
                configuration,
            )
        }
    }

    private fun handleDecoderState(decoderState: VideoDecoderState) {
        val configuration = activeConfiguration ?: return
        val hello = remoteHello ?: return
        when (decoderState) {
            is VideoDecoderState.AwaitingKeyframe,
            is VideoDecoderState.Running,
            -> {
                surfaceReady = true
                if (mutableState.value !is AndroidDisplaySessionState.Displaying) {
                    mutableState.value = AndroidDisplaySessionState.Connected(
                        hello.implementationName,
                        configuration,
                    )
                }
                drainPendingSurfaceFrames()
            }

            is VideoDecoderState.Recovering -> {
                surfaceReady = true
                mutableState.value = AndroidDisplaySessionState.Recovering(
                    attempt = decoderState.attempt,
                    reason = decoderState.reason,
                )
                drainPendingSurfaceFrames()
            }

            is VideoDecoderState.AwaitingSurface -> {
                surfaceReady = false
                surfaceOutputBaseline = decoder.metrics.value.outputsReleasedToSurface
                mutableState.value = AndroidDisplaySessionState.Configured(
                    hello.implementationName,
                    configuration,
                )
            }

            is VideoDecoderState.Failed -> {
                surfaceReady = false
                surfaceOutputBaseline = decoder.metrics.value.outputsReleasedToSurface
                mutableState.value = if (decoderState.recoverableOnKeyframe) {
                    AndroidDisplaySessionState.Recovering(1, decoderState.message)
                } else {
                    failed = true
                    AndroidDisplaySessionState.Failed(
                        reason = decoderState.message,
                        retryable = true,
                        kind = DisplaySessionFailureKind.Decoder,
                    )
                }
            }

            VideoDecoderState.Closed -> {
                surfaceReady = false
                failed = true
                mutableState.value = AndroidDisplaySessionState.Failed(
                    "Android decoder closed",
                    retryable = true,
                    kind = DisplaySessionFailureKind.Decoder,
                )
            }

            VideoDecoderState.AwaitingConfiguration,
            VideoDecoderState.Idle,
            -> Unit
        }
    }

    private fun enqueueUntilSurfaceReady(frame: LdflFrame) {
        val keyframe = frame.flags.contains(dev.ladoflow.display.protocol.FrameFlags.Keyframe)
        when {
            keyframe -> {
                recordDroppedVideo(pendingSurfaceFrames.size.toLong())
                pendingSurfaceFrames.clear()
                pendingSurfaceFrames.addLast(frame)
                awaitingSurfaceKeyframe = false
                updateQueueDepthMetric()
            }

            awaitingSurfaceKeyframe -> recordDroppedVideo()
            pendingSurfaceFrames.size >= 3 -> {
                recordDroppedVideo(pendingSurfaceFrames.size.toLong() + 1L)
                pendingSurfaceFrames.clear()
                awaitingSurfaceKeyframe = true
                updateQueueDepthMetric()
            }

            else -> {
                pendingSurfaceFrames.addLast(frame)
                updateQueueDepthMetric()
            }
        }
    }

    private fun drainPendingSurfaceFrames() {
        while (surfaceReady && pendingSurfaceFrames.isNotEmpty()) {
            if (!decoder.submit(pendingSurfaceFrames.removeFirst())) {
                recordDroppedVideo()
                pendingSurfaceFrames.clear()
                awaitingSurfaceKeyframe = true
                updateQueueDepthMetric()
                return
            }
        }
        updateQueueDepthMetric()
    }

    private suspend fun rejectProtocol(
        code: RemoteErrorCode,
        diagnostic: String,
    ) {
        activeConfiguration = null
        negotiated = false
        failed = true
        surfaceReady = false
        pendingSurfaceFrames.clear()
        updateQueueDepthMetric()
        inputController.updateViewport(null)
        decoder.reset(diagnostic)
        mutableState.value = AndroidDisplaySessionState.Failed(
            diagnostic,
            retryable = true,
            kind = DisplaySessionFailureKind.Protocol,
        )
        outbound?.sendControl(RemoteErrorPayload(code, retryable = true, diagnostic = diagnostic))
    }

    private fun failLocal(reason: String) {
        activeConfiguration = null
        negotiated = false
        failed = true
        surfaceReady = false
        pendingSurfaceFrames.clear()
        updateQueueDepthMetric()
        inputController.updateViewport(null)
        decoder.reset(reason)
        mutableState.value = AndroidDisplaySessionState.Failed(
            reason,
            retryable = false,
            kind = DisplaySessionFailureKind.Local,
        )
    }

    private fun resetProtocol(reason: String) {
        protocolGeneration += 1
        outbound?.close()
        outbound = null
        remoteHello = null
        remoteCapabilities = null
        negotiated = false
        failed = false
        activeConfiguration = null
        surfaceReady = false
        awaitingSurfaceKeyframe = true
        pendingSurfaceFrames.clear()
        inboundSequences = MonotonicSequenceValidator()
        localDroppedVideoFrames = 0L
        surfaceOutputBaseline = decoder.metrics.value.outputsReleasedToSurface
        mutableMetrics.value = AndroidDisplaySessionMetrics()
        lastReleasedFrameId = 0uL
        pingToken = 0uL
        inputController.updateViewport(null)
        if (mutableState.value is AndroidDisplaySessionState.Configured ||
            mutableState.value is AndroidDisplaySessionState.Connected ||
            mutableState.value is AndroidDisplaySessionState.Displaying ||
            mutableState.value is AndroidDisplaySessionState.Recovering ||
            mutableState.value is AndroidDisplaySessionState.Ready
        ) {
            decoder.reset(reason)
        }
    }

    private fun recordDroppedInput() {
        mutableMetrics.value = mutableMetrics.value.copy(
            droppedInputEvents = mutableMetrics.value.droppedInputEvents + 1,
        )
    }

    private fun recordDroppedVideo(count: Long = 1L) {
        if (count <= 0L) return
        localDroppedVideoFrames = localDroppedVideoFrames.saturatingAdd(count)
        mutableMetrics.value = mutableMetrics.value.copy(
            droppedVideoFrames = localDroppedVideoFrames.saturatingAdd(
                decoder.metrics.value.droppedFrames,
            ),
        )
    }

    private fun updateQueueDepthMetric() {
        val depth = (pendingSurfaceFrames.size + decoder.metrics.value.queueDepth)
            .coerceAtMost(MAX_TELEMETRY_QUEUE_DEPTH)
        if (mutableMetrics.value.queueDepth != depth) {
            mutableMetrics.value = mutableMetrics.value.copy(queueDepth = depth)
        }
    }

    private suspend fun telemetryLoop() {
        while (!closed.get()) {
            delay(1_000)
            sendTelemetryNow()
        }
    }

    internal suspend fun sendTelemetryNow(): Boolean {
        if (!negotiated || failed || activeConfiguration == null) return false
        val snapshot = mutableMetrics.value
        val currentOutbound = outbound ?: return false
        val queueDepth = (pendingSurfaceFrames.size + decoder.metrics.value.queueDepth)
            .coerceAtMost(MAX_TELEMETRY_QUEUE_DEPTH)
        if (snapshot.queueDepth != queueDepth) updateQueueDepthMetric()
        currentOutbound.sendControl(
            TelemetryPayload(
                sampleTimestampMicros = monotonicMicros(),
                frameId = lastReleasedFrameId,
                timings = StageTimings(
                    captureMicros = 0u,
                    encodeMicros = 0u,
                    transportMicros = 0u,
                    decodeMicros = snapshot.latestDecodeDurationMicros ?: 0u,
                    presentationMicros = 0u,
                ),
                queueDepth = queueDepth,
                lossPartsPerMillion = 0u,
                droppedFrames = snapshot.droppedVideoFrames
                    .coerceAtMost(UInt.MAX_VALUE.toLong()).toUInt(),
                lateFrames = 0u,
                thermalState = ThermalState.Unknown,
            ),
        )
        return true
    }

    private suspend fun pingLoop() {
        while (!closed.get()) {
            delay(2_000)
            if (!negotiated || failed || activeConfiguration == null) continue
            val token = pingToken
            pingToken = if (pingToken == ULong.MAX_VALUE) 0u else pingToken + 1u
            outbound?.sendControl(PingPayload(token, monotonicMicros()))
        }
    }

    override fun close() {
        if (!closed.compareAndSet(false, true)) return
        resetProtocol("Display session closed")
        surfaceController.release()
        sessionJob.cancel()
        mutableState.value = AndroidDisplaySessionState.Stopped
    }
}

private fun InputPayload.isSupportedBy(capabilities: InputCapabilities): Boolean = when (event) {
    is PointerMoveInput,
    is PointerButtonInput,
    is WheelInput,
    -> capabilities.contains(InputCapabilities.Pointer)

    is TouchInput -> capabilities.contains(InputCapabilities.Touch)
    is KeyInput -> capabilities.contains(InputCapabilities.Keyboard)
    is FocusInput -> capabilities != InputCapabilities.None
}

private fun secureNonce(): ByteArray = ByteArray(16).also(SecureRandom()::nextBytes)

private fun elapsedRealtimeMicros(): ULong =
    SystemClock.elapsedRealtimeNanos().toULong() / 1_000uL

private fun Long.saturatingAdd(value: Long): Long =
    if (value > Long.MAX_VALUE - this) Long.MAX_VALUE else this + value
