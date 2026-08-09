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

/**
 * Asynchronous H.264 decoder that serializes every MediaCodec call on one HandlerThread.
 *
 * It prefers a platform-reported hardware decoder, enables the official low-latency
 * feature only when the selected codec advertises it, and renders immediately to Surface.
 */
class AndroidMediaCodecVideoDecoder(
    private val maxPendingAccessUnits: Int = DEFAULT_PENDING_ACCESS_UNITS,
) : VideoDecoder {
    private val closed = AtomicBoolean(false)
    private val codecThread = HandlerThread("LadoFlow H264 decoder").apply { start() }
    private val codecHandler = Handler(codecThread.looper)
    private val gate = H264FrameGate()
    private val pendingInputs = ArrayDeque<H264DecoderInput>(maxPendingAccessUnits)
    private val availableInputBuffers = ArrayDeque<Int>()
    private val queuedFramesByTimestamp = mutableMapOf<Long, ArrayDeque<QueuedFrame>>()
    private val mutableState = MutableStateFlow<VideoDecoderState>(VideoDecoderState.Idle)
    private val mutableQueueDepth = MutableStateFlow(0)
    private val mutableEvents = MutableSharedFlow<VideoDecoderEvent>(
        extraBufferCapacity = 64,
        onBufferOverflow = BufferOverflow.DROP_OLDEST,
    )

    private var outputSurface: Surface? = null
    private var configuration: DisplayConfigPayload? = null
    private var activeCodec: MediaCodec? = null
    private var activeBackend: VideoDecoderBackend? = null

    override val state: StateFlow<VideoDecoderState> = mutableState.asStateFlow()
    override val events: SharedFlow<VideoDecoderEvent> = mutableEvents.asSharedFlow()
    override val queueDepth: StateFlow<Int> = mutableQueueDepth.asStateFlow()

    init {
        require(maxPendingAccessUnits > 0) { "Pending access-unit capacity must be positive" }
    }

    override fun setOutputSurface(surface: Surface?) {
        if (closed.get()) return
        codecHandler.post {
            if (closed.get() || outputSurface === surface) return@post
            releaseCodec()
            pendingInputs.clear()
            gate.awaitNextKeyframe()
            outputSurface = surface
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
            releaseCodec()
            pendingInputs.clear()
            gate.reset(clearParameterSets = true)
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
        return codecHandler.post {
            if (!closed.get()) processFrame(frame)
        }
    }

    override fun reset(reason: String) {
        if (closed.get()) return
        codecHandler.post {
            if (closed.get()) return@post
            releaseCodec()
            pendingInputs.clear()
            gate.awaitNextKeyframe()
            mutableEvents.tryEmit(VideoDecoderEvent.Warning("Decoder reset: $reason"))
            mutableState.value = when {
                outputSurface == null -> VideoDecoderState.AwaitingSurface(configuration)
                configuration == null -> VideoDecoderState.AwaitingConfiguration
                else -> VideoDecoderState.AwaitingKeyframe(requireNotNull(configuration))
            }
        }
    }

    private fun processFrame(frame: LdflFrame) {
        val currentConfiguration = configuration
        if (currentConfiguration == null || currentConfiguration.codec != VideoCodec.H264) {
            drop(null, DecoderDropReason.NoConfiguration, "No active H.264 DisplayConfig")
            return
        }
        if (outputSurface == null) {
            drop(null, DecoderDropReason.NoSurface, "No valid decoder output Surface")
            return
        }

        when (val decision = gate.offer(frame)) {
            is H264GateDecision.Dropped -> {
                drop(decision.frameId, decision.reason, decision.detail)
                if (
                    decision.reason == DecoderDropReason.AwaitingKeyframe ||
                    decision.reason == DecoderDropReason.MissingParameterSets ||
                    decision.reason == DecoderDropReason.ParameterSetsOnly
                ) {
                    mutableState.value = VideoDecoderState.AwaitingKeyframe(currentConfiguration)
                }
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
                    releaseCodec()
                    pendingInputs.clear()
                }
                if (pendingInputs.size >= maxPendingAccessUnits) {
                    handlePendingOverflow(decision, currentConfiguration)
                    return
                }
                pendingInputs.addLast(decision.input)
                updateQueueDepth()
                ensureCodec(currentConfiguration, decision.parameterSets)
                pumpInput()
            }
        }
    }

    private fun handlePendingOverflow(
        decision: H264GateDecision.Ready,
        currentConfiguration: DisplayConfigPayload,
    ) {
        val droppedCount = pendingInputs.size + 1
        pendingInputs.clear()
        releaseCodec()
        if (decision.input.isKeyframe) {
            pendingInputs.addLast(decision.input)
            updateQueueDepth()
            mutableEvents.tryEmit(
                VideoDecoderEvent.Warning(
                    "Decoder queue superseded $droppedCount access units with keyframe ${decision.input.frameId}",
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
            )
            mutableState.value = VideoDecoderState.AwaitingKeyframe(currentConfiguration)
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
            handleCodecFailure(exception.message ?: "Unable to configure Android MediaCodec")
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
                val queuedFrame = takeQueuedFrame(info.presentationTimeUs)
                if (render) {
                    codec.releaseOutputBuffer(index, System.nanoTime())
                    mutableEvents.tryEmit(
                        VideoDecoderEvent.OutputReleasedToSurface(
                            frameId = queuedFrame?.frameId,
                            presentationTimestampMicros = info.presentationTimeUs,
                        ),
                    )
                } else {
                    codec.releaseOutputBuffer(index, false)
                }
                if (endOfStream) {
                    mutableEvents.tryEmit(VideoDecoderEvent.EndOfStream)
                    pendingInputs.clear()
                    releaseCodec()
                    gate.awaitNextKeyframe()
                    configuration?.let { mutableState.value = VideoDecoderState.AwaitingKeyframe(it) }
                }
            } catch (exception: Exception) {
                handleCodecFailure(exception.message ?: "Unable to release MediaCodec output")
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
            handleCodecFailure(exception.diagnosticInfo.ifEmpty { "MediaCodec failed" })
        }
    }

    private fun pumpInput() {
        val codec = activeCodec ?: return
        while (availableInputBuffers.isNotEmpty() && pendingInputs.isNotEmpty()) {
            val inputIndex = availableInputBuffers.removeFirst()
            val input = pendingInputs.removeFirst()
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
                rememberQueuedFrame(input)
            } catch (exception: Exception) {
                handleCodecFailure(exception.message ?: "Unable to queue MediaCodec input")
                return
            }
        }
        updateQueueDepth()
    }

    private fun rememberQueuedFrame(input: H264DecoderInput) {
        queuedFramesByTimestamp
            .getOrPut(input.presentationTimestampMicros) { ArrayDeque() }
            .addLast(QueuedFrame(input.frameId))
    }

    private fun takeQueuedFrame(presentationTimestampMicros: Long): QueuedFrame? {
        val frames = queuedFramesByTimestamp[presentationTimestampMicros] ?: return null
        val frame = frames.pollFirst()
        if (frames.isEmpty()) queuedFramesByTimestamp.remove(presentationTimestampMicros)
        updateQueueDepth()
        return frame
    }

    private fun handleCodecFailure(message: String) {
        releaseCodec()
        pendingInputs.clear()
        gate.awaitNextKeyframe()
        mutableEvents.tryEmit(VideoDecoderEvent.Failure(message))
        mutableState.value = VideoDecoderState.Failed(
            message = message,
            recoverableOnKeyframe = true,
        )
    }

    private fun drop(
        frameId: ULong?,
        reason: DecoderDropReason,
        detail: String,
    ) {
        mutableEvents.tryEmit(VideoDecoderEvent.FrameDropped(frameId, reason, detail))
    }

    private fun releaseCodec() {
        val codec = activeCodec
        activeCodec = null
        activeBackend = null
        pendingInputs.clear()
        availableInputBuffers.clear()
        queuedFramesByTimestamp.clear()
        if (codec != null) {
            runCatching { codec.stop() }
            runCatching { codec.release() }
        }
        updateQueueDepth()
    }

    private fun updateQueueDepth() {
        mutableQueueDepth.value = pendingInputs.size +
            queuedFramesByTimestamp.values.sumOf { it.size }
    }

    override fun close() {
        if (!closed.compareAndSet(false, true)) return
        codecHandler.post {
            releaseCodec()
            pendingInputs.clear()
            outputSurface = null
            configuration = null
            mutableState.value = VideoDecoderState.Closed
            codecThread.quitSafely()
        }
    }

    private data class QueuedFrame(val frameId: ULong)

    companion object {
        private const val DEFAULT_PENDING_ACCESS_UNITS = 3
    }
}

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
