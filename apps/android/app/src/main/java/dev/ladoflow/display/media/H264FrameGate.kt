package dev.ladoflow.display.media

import dev.ladoflow.display.protocol.FrameFlags
import dev.ladoflow.display.protocol.LdflFrame
import dev.ladoflow.display.protocol.MessageType
import dev.ladoflow.display.protocol.PayloadCodec
import dev.ladoflow.display.protocol.VideoFramePayload

enum class DecoderDropReason {
    UnexpectedMessageType,
    InvalidAccessUnit,
    AwaitingKeyframe,
    MissingParameterSets,
    ParameterSetsOnly,
    TimestampOutOfRange,
    TimestampDiscontinuity,
    QueueOverflow,
    NoConfiguration,
    NoSurface,
    SurfaceInvalidated,
    ConfigurationChanged,
    KeyframeRecovery,
    OutputCorrelationLost,
    CodecFailure,
}

data class H264DecoderInput(
    val frameId: ULong,
    val presentationTimestampMicros: Long,
    val durationMicros: UInt,
    val accessUnit: ByteArray,
    val isKeyframe: Boolean,
    val endOfStream: Boolean,
    val containsIdr: Boolean,
)

sealed interface H264GateDecision {
    data class Ready(
        val input: H264DecoderInput,
        val parameterSets: H264ParameterSets,
        val requiresCodecRestart: Boolean,
    ) : H264GateDecision

    data class Dropped(
        val frameId: ULong?,
        val reason: DecoderDropReason,
        val detail: String,
    ) : H264GateDecision
}

/**
 * Protects a decoder from receiving a broken inter-frame chain.
 *
 * SPS/PPS NAL units may arrive before the first VCL access unit. After reset,
 * decoding resumes only on an LDFL KEYFRAME with both parameter sets available.
 */
class H264FrameGate {
    private var sequenceParameterSet: ByteArray? = null
    private var pictureParameterSet: ByteArray? = null
    private var awaitingKeyframe = true
    private var lastPresentationTimestampMicros: Long? = null

    fun reset(clearParameterSets: Boolean = true) {
        awaitingKeyframe = true
        lastPresentationTimestampMicros = null
        if (clearParameterSets) {
            sequenceParameterSet = null
            pictureParameterSet = null
        }
    }

    fun awaitNextKeyframe() {
        awaitingKeyframe = true
        lastPresentationTimestampMicros = null
    }

    fun offer(frame: LdflFrame): H264GateDecision {
        if (frame.messageType != MessageType.VideoFrame) {
            return H264GateDecision.Dropped(
                frameId = null,
                reason = DecoderDropReason.UnexpectedMessageType,
                detail = "Expected VideoFrame, received ${frame.messageType}",
            )
        }

        val payload = try {
            PayloadCodec.decodeAs<VideoFramePayload>(frame)
        } catch (exception: IllegalArgumentException) {
            return H264GateDecision.Dropped(
                frameId = null,
                reason = DecoderDropReason.InvalidAccessUnit,
                detail = exception.message ?: "Invalid VideoFrame payload",
            )
        }
        val inspection = try {
            H264AnnexB.inspect(payload.accessUnit)
        } catch (exception: H264AnnexBException) {
            return H264GateDecision.Dropped(
                frameId = payload.metadata.frameId,
                reason = DecoderDropReason.InvalidAccessUnit,
                detail = exception.message ?: "Invalid H.264 Annex-B access unit",
            )
        }

        var parameterSetsChanged = false
        inspection.sequenceParameterSet?.let { candidate ->
            if (sequenceParameterSet?.contentEquals(candidate) != true) {
                sequenceParameterSet = candidate.copyOf()
                parameterSetsChanged = true
            }
        }
        inspection.pictureParameterSet?.let { candidate ->
            if (pictureParameterSet?.contentEquals(candidate) != true) {
                pictureParameterSet = candidate.copyOf()
                parameterSetsChanged = true
            }
        }
        if (parameterSetsChanged && !awaitingKeyframe) {
            awaitingKeyframe = true
        }

        if (!inspection.containsVcl) {
            return H264GateDecision.Dropped(
                frameId = payload.metadata.frameId,
                reason = DecoderDropReason.ParameterSetsOnly,
                detail = "Buffered H.264 parameter data; waiting for a VCL keyframe",
            )
        }

        val isKeyframe = frame.flags.contains(FrameFlags.Keyframe)
        if (awaitingKeyframe && !isKeyframe) {
            return H264GateDecision.Dropped(
                frameId = payload.metadata.frameId,
                reason = DecoderDropReason.AwaitingKeyframe,
                detail = "Decoder is waiting for an LDFL KEYFRAME",
            )
        }

        val parameterSets = currentParameterSets()
        if (parameterSets == null) {
            awaitingKeyframe = true
            return H264GateDecision.Dropped(
                frameId = payload.metadata.frameId,
                reason = DecoderDropReason.MissingParameterSets,
                detail = "H.264 decoder needs Annex-B SPS and PPS before its first keyframe",
            )
        }

        val presentationTimestamp = payload.metadata.presentationTimestampMicros
        if (presentationTimestamp > Long.MAX_VALUE.toULong()) {
            return H264GateDecision.Dropped(
                frameId = payload.metadata.frameId,
                reason = DecoderDropReason.TimestampOutOfRange,
                detail = "Presentation timestamp does not fit Android MediaCodec's signed long",
            )
        }

        val signedPresentationTimestamp = presentationTimestamp.toLong()
        val timestampDiscontinuity = lastPresentationTimestampMicros?.let { previous ->
            signedPresentationTimestamp <= previous
        } == true
        if (timestampDiscontinuity) {
            awaitingKeyframe = true
            if (!isKeyframe) {
                lastPresentationTimestampMicros = null
                return H264GateDecision.Dropped(
                    frameId = payload.metadata.frameId,
                    reason = DecoderDropReason.TimestampDiscontinuity,
                    detail = "H.264 presentation timestamp did not increase; waiting for KEYFRAME",
                )
            }
        }

        val wasAwaitingKeyframe = awaitingKeyframe
        awaitingKeyframe = false
        lastPresentationTimestampMicros = signedPresentationTimestamp
        return H264GateDecision.Ready(
            input = H264DecoderInput(
                frameId = payload.metadata.frameId,
                presentationTimestampMicros = signedPresentationTimestamp,
                durationMicros = payload.metadata.durationMicros,
                accessUnit = payload.accessUnit.copyOf(),
                isKeyframe = isKeyframe,
                endOfStream = frame.flags.contains(FrameFlags.EndOfStream),
                containsIdr = inspection.containsIdr,
            ),
            parameterSets = parameterSets,
            requiresCodecRestart = wasAwaitingKeyframe || parameterSetsChanged,
        )
    }

    private fun currentParameterSets(): H264ParameterSets? {
        val sps = sequenceParameterSet ?: return null
        val pps = pictureParameterSet ?: return null
        return H264ParameterSets(sps, pps)
    }
}
