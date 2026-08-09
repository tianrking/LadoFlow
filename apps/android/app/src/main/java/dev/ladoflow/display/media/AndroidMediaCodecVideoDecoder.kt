package dev.ladoflow.display.media

import android.media.MediaCodec
import android.media.MediaCodecInfo
import android.media.MediaCodecList
import android.media.MediaFormat
import android.os.Build
import android.os.Handler
import android.os.HandlerThread
import android.view.Surface
import dev.ladoflow.display.protocol.DisplayConfigPayload
import dev.ladoflow.display.protocol.CodecProfile
import dev.ladoflow.display.protocol.LdflFrame
import dev.ladoflow.display.protocol.MessageType
import dev.ladoflow.display.protocol.VideoCodec
import java.nio.ByteBuffer
import java.util.ArrayDeque
import java.util.concurrent.atomic.AtomicBoolean
import kotlinx.coroutines.channels.BufferOverflow
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.SharedFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asSharedFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update

/**
 * Asynchronous H.264 decoder that serializes every MediaCodec call on one HandlerThread.
 *
 * It prefers a platform-reported hardware decoder, enables the official low-latency
 * feature only when the selected codec advertises it, and renders immediately to Surface.
 */
class AndroidMediaCodecVideoDecoder(
    private val maxInFlightAccessUnits: Int = DEFAULT_IN_FLIGHT_ACCESS_UNITS,
) : VideoDecoder {
    private val closed = AtomicBoolean(false)
    private val codecThread = HandlerThread("LadoFlow H264 decoder").apply { start() }
    private val codecHandler = Handler(codecThread.looper)
    private val gate = H264FrameGate()
    private val inFlightWindow = DecoderInFlightWindow(maxInFlightAccessUnits)
    private val pipeline = DecoderPipelineLedger(maxInFlightAccessUnits)
    private val availableInputBuffers = ArrayDeque<Int>()
    private val mutableState = MutableStateFlow<VideoDecoderState>(VideoDecoderState.Idle)
    private val mutableMetrics = MutableStateFlow(VideoDecoderMetrics())
    private val mutableEvents = MutableSharedFlow<VideoDecoderEvent>(
        extraBufferCapacity = 64,
        onBufferOverflow = BufferOverflow.DROP_OLDEST,
    )

    private var outputSurface: Surface? = null
    private var configuration: DisplayConfigPayload? = null
    private var activeCodec: MediaCodec? = null
    private var activeBackend: VideoDecoderBackend? = null
    private var consecutiveCodecFailures = 0
    private val submissionOverflowPending = AtomicBoolean(false)

    override val state: StateFlow<VideoDecoderState> = mutableState.asStateFlow()
    override val events: SharedFlow<VideoDecoderEvent> = mutableEvents.asSharedFlow()
    override val metrics: StateFlow<VideoDecoderMetrics> = mutableMetrics.asStateFlow()

    init {
        require(maxInFlightAccessUnits > 0) { "In-flight access-unit capacity must be positive" }
    }

    override fun setOutputSurface(surface: Surface?) {
        if (closed.get()) return
        codecHandler.post {
            if (closed.get() || outputSurface === surface) return@post
            discardCodecPipeline(
                DecoderDropReason.SurfaceInvalidated,
                "Decoder Surface changed; discarded in-flight access units",
            )
            gate.awaitNextKeyframe()
            outputSurface = surface
            consecutiveCodecFailures = 0
            mutableState.value = when {
                surface == null -> VideoDecoderState.AwaitingSurface(configuration)
                configuration == null -> VideoDecoderState.AwaitingConfiguration
                else -> VideoDecoderState.AwaitingKeyframe(requireNotNull(configuration))
            }
        }
    }

    override fun applyConfiguration(configuration: DisplayConfigPayload) {
        if (closed.get()) return
        codecHandler.post {
            if (closed.get()) return@post
            discardCodecPipeline(
                DecoderDropReason.ConfigurationChanged,
                "DisplayConfig changed; discarded in-flight access units",
            )
            gate.reset(clearParameterSets = true)
            consecutiveCodecFailures = 0
            this.configuration = configuration
            if (configuration.codec != VideoCodec.H264) {
                mutableState.value = VideoDecoderState.Failed(
                    message = "${configuration.codec} decode is not enabled in the Android v1 boundary",
                    recoverableOnKeyframe = false,
                )
                mutableEvents.tryEmit(
                    VideoDecoderEvent.Failure("Unsupported decoder codec ${configuration.codec}"),
                )
            } else if (outputSurface == null) {
                mutableState.value = VideoDecoderState.AwaitingSurface(configuration)
            } else {
                mutableState.value = VideoDecoderState.AwaitingKeyframe(configuration)
            }
        }
    }

    override fun configurationRejectionReason(configuration: DisplayConfigPayload): String? =
        runCatching { selectAvcDecoder(configuration) }.exceptionOrNull()?.message

    override fun submit(frame: LdflFrame): Boolean {
        if (closed.get() || frame.messageType != MessageType.VideoFrame) return false
        if (!inFlightWindow.tryAcquire()) {
            scheduleSubmissionOverflowRecovery()
            return false
        }
        updateQueueDepth()
        val posted = codecHandler.post {
            if (closed.get()) {
                releaseInFlight()
            } else {
                processSubmittedFrame(frame)
            }
        }
        if (!posted) releaseInFlight()
        return posted
    }

    override fun reset(reason: String) {
        if (closed.get()) return
        codecHandler.post {
            if (closed.get()) return@post
            releaseCodec()
            gate.awaitNextKeyframe()
            consecutiveCodecFailures = 0
            mutableMetrics.value = VideoDecoderMetrics()
            mutableEvents.tryEmit(VideoDecoderEvent.Warning("Decoder reset: $reason"))
            mutableState.value = when {
                outputSurface == null -> VideoDecoderState.AwaitingSurface(configuration)
                configuration == null -> VideoDecoderState.AwaitingConfiguration
                else -> VideoDecoderState.AwaitingKeyframe(requireNotNull(configuration))
            }
        }
    }

    private fun processSubmittedFrame(frame: LdflFrame) {
        val retainedByPipeline = try {
            processFrame(frame)
        } catch (exception: Exception) {
            mutableEvents.tryEmit(
                VideoDecoderEvent.Failure(
                    exception.message ?: "Unexpected decoder submission failure",
                ),
            )
            false
        }
        if (!retainedByPipeline) releaseInFlight()
    }

    /** Returns true once the caller's in-flight reservation belongs to [pipeline]. */
    private fun processFrame(frame: LdflFrame): Boolean {
        val currentConfiguration = configuration
        if (currentConfiguration == null || currentConfiguration.codec != VideoCodec.H264) {
            drop(null, DecoderDropReason.NoConfiguration, "No active H.264 DisplayConfig")
            return false
        }
        if (outputSurface == null) {
            drop(null, DecoderDropReason.NoSurface, "No valid decoder output Surface")
            return false
        }

        return when (val decision = gate.offer(frame)) {
            is H264GateDecision.Dropped -> {
                drop(
                    frameId = decision.frameId,
                    reason = decision.reason,
                    detail = decision.detail,
                    count = if (decision.reason.countsAsVideoDrop()) 1 else 0,
                )
                if (
                    decision.reason == DecoderDropReason.AwaitingKeyframe ||
                    decision.reason == DecoderDropReason.MissingParameterSets ||
                    decision.reason == DecoderDropReason.ParameterSetsOnly
                ) {
                    mutableState.value = VideoDecoderState.AwaitingKeyframe(currentConfiguration)
                } else if (decision.reason == DecoderDropReason.TimestampDiscontinuity) {
                    mutableState.value = VideoDecoderState.Recovering(
                        configuration = currentConfiguration,
                        attempt = 1,
                        reason = decision.detail,
                    )
                }
                false
            }

            is H264GateDecision.Ready -> {
                if (decision.input.isKeyframe && !decision.input.containsIdr) {
                    mutableEvents.tryEmit(
                        VideoDecoderEvent.Warning(
                            "LDFL KEYFRAME ${decision.input.frameId} contains no H.264 IDR NAL",
                        ),
                    )
                }
                if (decision.requiresCodecRestart) {
                    discardCodecPipeline(
                        DecoderDropReason.KeyframeRecovery,
                        "Keyframe restarted the decoder and superseded older access units",
                    )
                }
                if (!pipeline.enqueue(decision.input)) {
                    handlePipelineOverflow(decision, currentConfiguration)
                    return decision.input.isKeyframe
                }
                updateQueueDepth()
                ensureCodec(currentConfiguration, decision.parameterSets)
                pumpInput()
                true
            }
        }
    }

    private fun scheduleSubmissionOverflowRecovery() {
        if (!submissionOverflowPending.compareAndSet(false, true)) return
        if (!codecHandler.post {
                submissionOverflowPending.set(false)
                if (closed.get()) return@post
                val currentConfiguration = configuration
                gate.awaitNextKeyframe()
                mutableEvents.tryEmit(
                    VideoDecoderEvent.Warning(
                        "Decoder admission window overflowed; preserving the accepted prefix " +
                            "and waiting for a fresh keyframe",
                    ),
                )
                mutableState.value = when {
                    currentConfiguration == null -> VideoDecoderState.AwaitingConfiguration
                    outputSurface == null -> VideoDecoderState.AwaitingSurface(currentConfiguration)
                    else -> VideoDecoderState.Recovering(
                        configuration = currentConfiguration,
                        attempt = 1,
                        reason = "Decoder queue overflowed; waiting for a fresh keyframe",
                    )
                }
            }
        ) {
            submissionOverflowPending.set(false)
        }
    }

    private fun handlePipelineOverflow(
        decision: H264GateDecision.Ready,
        currentConfiguration: DisplayConfigPayload,
    ) {
        val previouslyInFlight = releaseCodec()
        if (decision.input.isKeyframe) {
            drop(
                frameId = null,
                reason = DecoderDropReason.QueueOverflow,
                detail = "Keyframe ${decision.input.frameId} superseded $previouslyInFlight in-flight access units",
                count = previouslyInFlight,
            )
            check(pipeline.enqueue(decision.input))
            updateQueueDepth()
            mutableEvents.tryEmit(
                VideoDecoderEvent.Warning(
                    "Decoder queue recovered on keyframe ${decision.input.frameId}",
                ),
            )
            ensureCodec(currentConfiguration, decision.parameterSets)
            pumpInput()
        } else {
            gate.awaitNextKeyframe()
            drop(
                decision.input.frameId,
                DecoderDropReason.QueueOverflow,
                "Decoder queue overflowed; dropped the dependent chain and now awaits KEYFRAME",
                count = previouslyInFlight + 1,
            )
            mutableState.value = VideoDecoderState.Recovering(
                configuration = currentConfiguration,
                attempt = 1,
                reason = "Decoder queue overflowed; waiting for a fresh keyframe",
            )
        }
    }

    private fun ensureCodec(
        currentConfiguration: DisplayConfigPayload,
        parameterSets: H264ParameterSets,
    ) {
        if (activeCodec != null) return
        val surface = outputSurface ?: return
        try {
            val selection = selectAvcDecoder(currentConfiguration)
            val format = createMediaFormat(currentConfiguration, parameterSets, selection.lowLatencySupported)
            val codec = MediaCodec.createByCodecName(selection.codecName)
            codec.setCallback(codecCallback, codecHandler)
            codec.configure(format, surface, null, 0)
            activeCodec = codec
            activeBackend = VideoDecoderBackend(
                codecName = selection.codecName,
                hardwareAcceleration = selection.hardwareAcceleration,
                lowLatencyFeatureEnabled = selection.lowLatencySupported,
            )
            codec.start()
            mutableState.value = VideoDecoderState.Running(
                configuration = currentConfiguration,
                backend = requireNotNull(activeBackend),
            )
        } catch (exception: Exception) {
            handleCodecFailure(
                exception.message ?: "Unable to configure Android MediaCodec",
                DecoderDropReason.CodecFailure,
            )
        }
    }

    private val codecCallback = object : MediaCodec.Callback() {
        override fun onInputBufferAvailable(codec: MediaCodec, index: Int) {
            if (activeCodec !== codec) return
            availableInputBuffers.addLast(index)
            pumpInput()
        }

        override fun onOutputBufferAvailable(
            codec: MediaCodec,
            index: Int,
            info: MediaCodec.BufferInfo,
        ) {
            if (activeCodec !== codec) return
            try {
                val codecConfig = info.flags and MediaCodec.BUFFER_FLAG_CODEC_CONFIG != 0
                val endOfStream = info.flags and MediaCodec.BUFFER_FLAG_END_OF_STREAM != 0
                val render = !codecConfig && info.size > 0
                if (render) {
                    val outputAvailableAtNanos = System.nanoTime()
                    val correlated = pipeline.takeOutput(
                        presentationTimestampMicros = info.presentationTimeUs,
                        outputAvailableAtNanos = outputAvailableAtNanos,
                    )
                    if (correlated == null) {
                        codec.releaseOutputBuffer(index, false)
                        handleCodecFailure(
                            "MediaCodec output timestamp ${info.presentationTimeUs} has no queued Host frame",
                            DecoderDropReason.OutputCorrelationLost,
                        )
                        return
                    }
                    codec.releaseOutputBuffer(index, true)
                    releaseInFlight()
                    recordOutput(correlated)
                    mutableEvents.tryEmit(
                        VideoDecoderEvent.OutputReleasedToSurface(
                            frameId = correlated.frameId,
                            presentationTimestampMicros = info.presentationTimeUs,
                            decodeDurationMicros = correlated.decodeDurationMicros,
                        ),
                    )
                } else {
                    codec.releaseOutputBuffer(index, false)
                }
                if (endOfStream) {
                    mutableEvents.tryEmit(VideoDecoderEvent.EndOfStream)
                    discardCodecPipeline(
                        DecoderDropReason.CodecFailure,
                        "End of stream discarded trailing in-flight access units",
                    )
                    gate.awaitNextKeyframe()
                    configuration?.let { mutableState.value = VideoDecoderState.AwaitingKeyframe(it) }
                }
            } catch (exception: Exception) {
                handleCodecFailure(
                    exception.message ?: "Unable to release MediaCodec output",
                    DecoderDropReason.CodecFailure,
                )
            }
        }

        override fun onOutputFormatChanged(codec: MediaCodec, format: MediaFormat) {
            if (activeCodec !== codec) return
            mutableEvents.tryEmit(
                VideoDecoderEvent.OutputFormatChanged(
                    width = format.integerOrNull(MediaFormat.KEY_WIDTH),
                    height = format.integerOrNull(MediaFormat.KEY_HEIGHT),
                ),
            )
        }

        override fun onError(codec: MediaCodec, exception: MediaCodec.CodecException) {
            if (activeCodec !== codec) return
            handleCodecFailure(
                exception.diagnosticInfo.ifEmpty { "MediaCodec failed" },
                DecoderDropReason.CodecFailure,
            )
        }
    }

    private fun pumpInput() {
        val codec = activeCodec ?: return
        while (availableInputBuffers.isNotEmpty()) {
            val input = pipeline.takePending() ?: break
            val inputIndex = availableInputBuffers.removeFirst()
            try {
                val inputBuffer = codec.getInputBuffer(inputIndex)
                    ?: throw IllegalStateException("MediaCodec returned no input buffer")
                inputBuffer.clear()
                if (input.accessUnit.size > inputBuffer.remaining()) {
                    throw IllegalArgumentException(
                        "Encoded access unit is ${input.accessUnit.size} bytes but codec input capacity is " +
                            "${inputBuffer.remaining()} bytes",
                    )
                }
                inputBuffer.put(input.accessUnit)
                val flags = if (input.endOfStream) MediaCodec.BUFFER_FLAG_END_OF_STREAM else 0
                codec.queueInputBuffer(
                    inputIndex,
                    0,
                    input.accessUnit.size,
                    input.presentationTimestampMicros,
                    flags,
                )
                pipeline.markQueued(input, System.nanoTime())
            } catch (exception: Exception) {
                handleCodecFailure(
                    exception.message ?: "Unable to queue MediaCodec input",
                    DecoderDropReason.CodecFailure,
                    additionallyDropped = 1,
                )
                return
            }
        }
        updateQueueDepth()
    }

    private fun handleCodecFailure(
        message: String,
        reason: DecoderDropReason,
        additionallyDropped: Int = 0,
    ) {
        val discarded = releaseCodec() + additionallyDropped
        drop(
            frameId = null,
            reason = reason,
            detail = message,
            count = discarded,
        )
        gate.awaitNextKeyframe()
        mutableEvents.tryEmit(VideoDecoderEvent.Failure(message))
        consecutiveCodecFailures += 1
        val currentConfiguration = configuration
        mutableState.value = if (
            consecutiveCodecFailures <= MAX_CONSECUTIVE_CODEC_FAILURES &&
            currentConfiguration != null &&
            outputSurface != null
        ) {
            VideoDecoderState.Recovering(
                configuration = currentConfiguration,
                attempt = consecutiveCodecFailures,
                reason = message,
            )
        } else {
            VideoDecoderState.Failed(
                message = message,
                recoverableOnKeyframe = false,
            )
        }
    }

    private fun drop(
        frameId: ULong?,
        reason: DecoderDropReason,
        detail: String,
        count: Int = 1,
    ) {
        require(count >= 0)
        if (count > 0) {
            mutableMetrics.update { metrics ->
                metrics.copy(
                    droppedFrames = metrics.droppedFrames.saturatingAdd(count.toLong()),
                    queueDepth = inFlightWindow.depth,
                )
            }
        }
        mutableEvents.tryEmit(
            VideoDecoderEvent.FrameDropped(
                frameId = frameId,
                reason = reason,
                detail = detail,
                count = count.toUInt(),
            ),
        )
    }

    private fun recordOutput(output: CorrelatedDecoderOutput) {
        consecutiveCodecFailures = 0
        mutableMetrics.update { metrics ->
            metrics.copy(
                outputsReleasedToSurface = metrics.outputsReleasedToSurface.saturatingAdd(1),
                queueDepth = inFlightWindow.depth,
                lastReleasedFrameId = output.frameId,
                lastDecodeDurationMicros = output.decodeDurationMicros,
            )
        }
    }

    private fun discardCodecPipeline(
        reason: DecoderDropReason,
        detail: String,
    ): Int {
        val discarded = releaseCodec()
        if (discarded > 0) {
            drop(frameId = null, reason = reason, detail = detail, count = discarded)
        }
        return discarded
    }

    private fun releaseCodec(): Int {
        val codec = activeCodec
        activeCodec = null
        activeBackend = null
        availableInputBuffers.clear()
        val discarded = pipeline.discardAll()
        releaseInFlight(discarded)
        if (codec != null) {
            runCatching { codec.stop() }
            runCatching { codec.release() }
        }
        updateQueueDepth()
        return discarded
    }

    private fun updateQueueDepth() {
        val depth = inFlightWindow.depth
        mutableMetrics.update { metrics ->
            if (metrics.queueDepth == depth) metrics else metrics.copy(queueDepth = depth)
        }
    }

    private fun releaseInFlight(count: Int = 1) {
        inFlightWindow.release(count)
        updateQueueDepth()
    }

    override fun close() {
        if (!closed.compareAndSet(false, true)) return
        codecHandler.post {
            releaseCodec()
            outputSurface = null
            configuration = null
            mutableState.value = VideoDecoderState.Closed
            codecThread.quitSafely()
        }
    }

    companion object {
        private const val DEFAULT_IN_FLIGHT_ACCESS_UNITS = 8
        private const val MAX_CONSECUTIVE_CODEC_FAILURES = 3
    }
}

private fun DecoderDropReason.countsAsVideoDrop(): Boolean = when (this) {
    DecoderDropReason.UnexpectedMessageType,
    DecoderDropReason.ParameterSetsOnly,
    -> false

    else -> true
}

private fun Long.saturatingAdd(value: Long): Long =
    if (value > Long.MAX_VALUE - this) Long.MAX_VALUE else this + value

private data class DecoderSelection(
    val codecName: String,
    val hardwareAcceleration: HardwareAccelerationEvidence,
    val lowLatencySupported: Boolean,
)

private fun selectAvcDecoder(configuration: DisplayConfigPayload): DecoderSelection {
    require(configuration.codec == VideoCodec.H264) {
        "Android v1 MediaCodec boundary supports H.264 only"
    }
    require(configuration.profile == CodecProfile.H264Main) {
        "Android v1 MediaCodec boundary supports H.264 Main only"
    }
    val candidates = MediaCodecList(MediaCodecList.REGULAR_CODECS).codecInfos
        .asSequence()
        .filterNot { it.isEncoder }
        .filter { info -> info.supportedTypes.any { it.equals(MediaFormat.MIMETYPE_VIDEO_AVC, true) } }
        .mapNotNull { info ->
            val capabilities = runCatching {
                info.getCapabilitiesForType(MediaFormat.MIMETYPE_VIDEO_AVC)
            }.getOrNull() ?: return@mapNotNull null
            if (!capabilities.supportsH264Main(configuration)) return@mapNotNull null

            val evidence = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
                when {
                    info.isHardwareAccelerated -> HardwareAccelerationEvidence.ReportedHardware
                    info.isSoftwareOnly -> HardwareAccelerationEvidence.ReportedSoftware
                    else -> HardwareAccelerationEvidence.NotReportedByPlatform
                }
            } else {
                HardwareAccelerationEvidence.NotReportedByPlatform
            }
            val lowLatency = Build.VERSION.SDK_INT >= Build.VERSION_CODES.R &&
                capabilities.isFeatureSupported(MediaCodecInfo.CodecCapabilities.FEATURE_LowLatency)
            DecoderSelection(info.name, evidence, lowLatency)
        }
        .sortedWith(
            compareBy<DecoderSelection> { it.hardwareAcceleration.selectionRank() }
                .thenBy { it.codecName },
        )
        .toList()

    return candidates.firstOrNull()
        ?: throw IllegalStateException(
            "No Android H.264 Main decoder supports ${configuration.width}x${configuration.height} " +
                "at ${configuration.refreshMillihz} mHz and ${configuration.bitrateKbps} kbps",
        )
}

private fun MediaCodecInfo.CodecCapabilities.supportsH264Main(
    configuration: DisplayConfigPayload,
): Boolean {
    if (profileLevels.none { it.profile == MediaCodecInfo.CodecProfileLevel.AVCProfileMain }) {
        return false
    }
    val video = videoCapabilities ?: return false
    val refreshHertz = configuration.refreshMillihz.toDouble() / 1_000.0
    val sizeAndRateSupported = runCatching {
        video.areSizeAndRateSupported(configuration.width, configuration.height, refreshHertz)
    }.getOrDefault(false)
    if (!sizeAndRateSupported) return false
    val bitrateBitsPerSecond = configuration.bitrateKbps.toLong() * 1_000L
    return bitrateBitsPerSecond <= Int.MAX_VALUE &&
        video.bitrateRange.contains(bitrateBitsPerSecond.toInt())
}

private fun createMediaFormat(
    configuration: DisplayConfigPayload,
    parameterSets: H264ParameterSets,
    lowLatencySupported: Boolean,
): MediaFormat = MediaFormat.createVideoFormat(
    MediaFormat.MIMETYPE_VIDEO_AVC,
    configuration.width,
    configuration.height,
).apply {
    setByteBuffer("csd-0", ByteBuffer.wrap(parameterSets.sequenceParameterSetCsd()))
    setByteBuffer("csd-1", ByteBuffer.wrap(parameterSets.pictureParameterSetCsd()))
    setInteger(MediaFormat.KEY_PROFILE, MediaCodecInfo.CodecProfileLevel.AVCProfileMain)
    setFloat(MediaFormat.KEY_FRAME_RATE, configuration.refreshMillihz.toFloat() / 1_000f)
    setFloat(MediaFormat.KEY_OPERATING_RATE, configuration.refreshMillihz.toFloat() / 1_000f)
    setInteger(MediaFormat.KEY_BIT_RATE, configuration.bitrateKbps.toLong().times(1_000L).toInt())
    setInteger(MediaFormat.KEY_PRIORITY, 0)
    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R && lowLatencySupported) {
        setInteger(MediaFormat.KEY_LOW_LATENCY, 1)
    }
}

private fun HardwareAccelerationEvidence.selectionRank(): Int = when (this) {
    HardwareAccelerationEvidence.ReportedHardware -> 0
    HardwareAccelerationEvidence.NotReportedByPlatform -> 1
    HardwareAccelerationEvidence.ReportedSoftware -> 2
}

private fun MediaFormat.integerOrNull(key: String): Int? =
    runCatching { getInteger(key) }.getOrNull()
