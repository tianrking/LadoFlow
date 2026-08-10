package dev.ladoflow.display.media

import dev.ladoflow.display.protocol.FrameFlags
import dev.ladoflow.display.protocol.LdflFrame
import dev.ladoflow.display.protocol.MessageType
import dev.ladoflow.display.protocol.VideoFrameMetadata
import dev.ladoflow.display.protocol.VideoFramePayload
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class H264FrameGateTest {
    private val parameterNalUnits = byteArrayOf(
        0, 0, 0, 1, 0x67, 0x42, 0x00, 0x1f,
        0, 0, 0, 1, 0x68, 0x01, 0x02,
    )
    private val idrNalUnit = byteArrayOf(0, 0, 0, 1, 0x65, 0x11, 0x22)
    private val deltaNalUnit = byteArrayOf(0, 0, 0, 1, 0x41, 0x33, 0x44)

    @Test
    fun firstDecodeRequiresParameterSetsAndLdflKeyframe() {
        val gate = H264FrameGate()

        val delta = gate.offer(videoFrame(1u, deltaNalUnit, FrameFlags.None))
        assertDropped(delta, DecoderDropReason.AwaitingKeyframe)

        val keyframeWithoutCsd = gate.offer(videoFrame(2u, idrNalUnit, FrameFlags.Keyframe))
        assertDropped(keyframeWithoutCsd, DecoderDropReason.MissingParameterSets)

        val ready = gate.offer(
            videoFrame(3u, parameterNalUnits + idrNalUnit, FrameFlags.Keyframe),
        ) as H264GateDecision.Ready
        assertTrue(ready.requiresCodecRestart)
        assertTrue(ready.input.isKeyframe)
        assertTrue(ready.input.containsIdr)
        assertEquals(3uL, ready.input.frameId)
    }

    @Test
    fun parameterOnlyUnitsPrimeTheNextKeyframe() {
        val gate = H264FrameGate()

        val primed = gate.offer(videoFrame(10u, parameterNalUnits, FrameFlags.None))
        assertDropped(primed, DecoderDropReason.ParameterSetsOnly)

        val ready = gate.offer(videoFrame(11u, idrNalUnit, FrameFlags.Keyframe))
            as H264GateDecision.Ready
        assertTrue(ready.requiresCodecRestart)
        assertEquals(11uL, ready.input.frameId)
    }

    @Test
    fun dependentFramesFlowOnlyAfterKeyframe() {
        val gate = H264FrameGate()
        gate.offer(videoFrame(20u, parameterNalUnits + idrNalUnit, FrameFlags.Keyframe))

        val delta = gate.offer(videoFrame(21u, deltaNalUnit, FrameFlags.None))
            as H264GateDecision.Ready

        assertFalse(delta.requiresCodecRestart)
        assertFalse(delta.input.isKeyframe)
        assertEquals(21uL, delta.input.frameId)
    }

    @Test
    fun decoderIdentityComesFromHostVideoMetadataFrameId() {
        val gate = H264FrameGate()
        val ready = gate.offer(
            videoFrame(
                frameId = 0x1234u,
                bytes = parameterNalUnits + idrNalUnit,
                flags = FrameFlags.Keyframe,
                sequence = 99u,
            ),
        ) as H264GateDecision.Ready

        assertEquals(0x1234uL, ready.input.frameId)
    }

    @Test
    fun resetKeepsCsdButRequiresAnotherKeyframeWhenRequested() {
        val gate = H264FrameGate()
        gate.offer(videoFrame(30u, parameterNalUnits + idrNalUnit, FrameFlags.Keyframe))
        gate.awaitNextKeyframe()

        assertDropped(
            gate.offer(videoFrame(31u, deltaNalUnit, FrameFlags.None)),
            DecoderDropReason.AwaitingKeyframe,
        )
        val recovered = gate.offer(videoFrame(32u, idrNalUnit, FrameFlags.Keyframe))
            as H264GateDecision.Ready
        assertTrue(recovered.requiresCodecRestart)
    }

    @Test
    fun changedParameterSetsBreakTheOldDependentChain() {
        val gate = H264FrameGate()
        gate.offer(videoFrame(40u, parameterNalUnits + idrNalUnit, FrameFlags.Keyframe))
        val changedSps = byteArrayOf(0, 0, 1, 0x67, 0x64, 0x00, 0x20)

        assertDropped(
            gate.offer(videoFrame(41u, changedSps + deltaNalUnit, FrameFlags.None)),
            DecoderDropReason.AwaitingKeyframe,
        )
        val restarted = gate.offer(videoFrame(42u, idrNalUnit, FrameFlags.Keyframe))
            as H264GateDecision.Ready
        assertTrue(restarted.requiresCodecRestart)
    }

    @Test
    fun ldflKeyframeFlagIsAuthoritativeButMissingIdrIsVisible() {
        val gate = H264FrameGate()
        val ready = gate.offer(
            videoFrame(50u, parameterNalUnits + deltaNalUnit, FrameFlags.Keyframe),
        ) as H264GateDecision.Ready

        assertTrue(ready.input.isKeyframe)
        assertFalse(ready.input.containsIdr)
    }

    @Test
    fun rejectsWrongMessageAndNonAnnexBPayload() {
        val gate = H264FrameGate()
        val wrong = LdflFrame(MessageType.Ping, FrameFlags.None, 1u, ByteArray(16))
        assertDropped(gate.offer(wrong), DecoderDropReason.UnexpectedMessageType)

        assertDropped(
            gate.offer(videoFrame(60u, byteArrayOf(1, 2, 3), FrameFlags.Keyframe)),
            DecoderDropReason.InvalidAccessUnit,
        )
    }

    @Test
    fun rejectsUnsignedTimestampOutsideMediaCodecRange() {
        val gate = H264FrameGate()
        val payload = VideoFramePayload(
            metadata = VideoFrameMetadata(
                frameId = 70u,
                captureTimestampMicros = 1u,
                presentationTimestampMicros = Long.MAX_VALUE.toULong() + 1u,
                durationMicros = 16_667u,
            ),
            accessUnit = parameterNalUnits + idrNalUnit,
        )
        val frame = LdflFrame.fromPayload(FrameFlags.Keyframe, 70u, payload)

        assertDropped(gate.offer(frame), DecoderDropReason.TimestampOutOfRange)
    }

    @Test
    fun timestampDiscontinuityDropsDependentChainUntilKeyframe() {
        val gate = H264FrameGate()
        gate.offer(
            videoFrame(
                frameId = 80u,
                bytes = parameterNalUnits + idrNalUnit,
                flags = FrameFlags.Keyframe,
                presentationTimestampMicros = 80_000u,
            ),
        )

        assertDropped(
            gate.offer(
                videoFrame(
                    frameId = 81u,
                    bytes = deltaNalUnit,
                    flags = FrameFlags.None,
                    presentationTimestampMicros = 79_000u,
                ),
            ),
            DecoderDropReason.TimestampDiscontinuity,
        )
        assertDropped(
            gate.offer(
                videoFrame(
                    frameId = 82u,
                    bytes = deltaNalUnit,
                    flags = FrameFlags.None,
                    presentationTimestampMicros = 81_000u,
                ),
            ),
            DecoderDropReason.AwaitingKeyframe,
        )
        val recovered = gate.offer(
            videoFrame(
                frameId = 83u,
                bytes = parameterNalUnits + idrNalUnit,
                flags = FrameFlags.Keyframe,
                presentationTimestampMicros = 82_000u,
            ),
        ) as H264GateDecision.Ready

        assertTrue(recovered.requiresCodecRestart)
        assertEquals(83uL, recovered.input.frameId)
    }

    private fun videoFrame(
        frameId: ULong,
        bytes: ByteArray,
        flags: FrameFlags,
        sequence: ULong = frameId,
        presentationTimestampMicros: ULong = frameId * 10u + 5u,
    ): LdflFrame = LdflFrame.fromPayload(
        flags = flags,
        sequence = sequence,
        payload = VideoFramePayload(
            metadata = VideoFrameMetadata(
                frameId = frameId,
                captureTimestampMicros = frameId * 10u,
                presentationTimestampMicros = presentationTimestampMicros,
                durationMicros = 16_667u,
            ),
            accessUnit = bytes,
        ),
    )

    private fun assertDropped(
        decision: H264GateDecision,
        expectedReason: DecoderDropReason,
    ) {
        assertEquals(expectedReason, (decision as H264GateDecision.Dropped).reason)
    }
}
