package dev.ladoflow.display.media

import android.view.Surface
import dev.ladoflow.display.protocol.DisplayConfigPayload
import dev.ladoflow.display.protocol.LdflFrame
import java.io.Closeable
import kotlinx.coroutines.flow.SharedFlow
import kotlinx.coroutines.flow.StateFlow

enum class HardwareAccelerationEvidence {
    ReportedHardware,
    ReportedSoftware,
    NotReportedByPlatform,
}

data class VideoDecoderBackend(
    val codecName: String,
    val hardwareAcceleration: HardwareAccelerationEvidence,
    val lowLatencyFeatureEnabled: Boolean,
)

sealed interface VideoDecoderState {
    data object Idle : VideoDecoderState

    data class AwaitingSurface(val configuration: DisplayConfigPayload?) : VideoDecoderState

    data object AwaitingConfiguration : VideoDecoderState

    data class AwaitingKeyframe(val configuration: DisplayConfigPayload) : VideoDecoderState

    data class Running(
        val configuration: DisplayConfigPayload,
        val backend: VideoDecoderBackend,
    ) : VideoDecoderState

    data class Recovering(
        val configuration: DisplayConfigPayload,
        val attempt: Int,
        val reason: String,
    ) : VideoDecoderState

    data class Failed(
        val message: String,
        val recoverableOnKeyframe: Boolean,
    ) : VideoDecoderState

    data object Closed : VideoDecoderState
}

sealed interface VideoDecoderEvent {
    data class FrameDropped(
        val frameId: ULong?,
        val reason: DecoderDropReason,
        val detail: String,
        val count: UInt = 1u,
    ) : VideoDecoderEvent

    /** MediaCodec released the decoded buffer to Surface; physical presentation is not implied. */
    data class OutputReleasedToSurface(
        val frameId: ULong,
        val presentationTimestampMicros: Long,
        val decodeDurationMicros: UInt,
    ) : VideoDecoderEvent

    data class OutputFormatChanged(
        val width: Int?,
        val height: Int?,
    ) : VideoDecoderEvent

    data class Warning(val message: String) : VideoDecoderEvent

    data class Failure(val message: String) : VideoDecoderEvent

    data object EndOfStream : VideoDecoderEvent
}

/** Authoritative cumulative decoder snapshot; diagnostic events may be best-effort. */
data class VideoDecoderMetrics(
    val outputsReleasedToSurface: Long = 0,
    val droppedFrames: Long = 0,
    val queueDepth: Int = 0,
    val lastReleasedFrameId: ULong? = null,
    val lastDecodeDurationMicros: UInt? = null,
)

/** Codec-neutral lifecycle boundary. Version one currently enables only H.264. */
interface VideoDecoder : Closeable {
    val state: StateFlow<VideoDecoderState>
    val events: SharedFlow<VideoDecoderEvent>
    /** Access units pending input plus codec inputs awaiting output release. */
    val metrics: StateFlow<VideoDecoderMetrics>

    /** The caller owns [surface] and must send null before it becomes invalid. */
    fun setOutputSurface(surface: Surface?)

    /** Returns a diagnostic when this concrete decoder cannot honor the exact config. */
    fun configurationRejectionReason(configuration: DisplayConfigPayload): String? = null

    fun applyConfiguration(configuration: DisplayConfigPayload)

    /** Returns false only when the decoder is closed or cannot enqueue the command. */
    fun submit(frame: LdflFrame): Boolean

    fun reset(reason: String)
}
