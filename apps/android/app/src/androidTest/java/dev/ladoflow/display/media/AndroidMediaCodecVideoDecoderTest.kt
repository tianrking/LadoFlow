package dev.ladoflow.display.media

import android.graphics.SurfaceTexture
import android.util.Base64
import android.view.Surface
import androidx.test.ext.junit.runners.AndroidJUnit4
import dev.ladoflow.display.protocol.CodecProfile
import dev.ladoflow.display.protocol.DisplayConfigPayload
import dev.ladoflow.display.protocol.FrameFlags
import dev.ladoflow.display.protocol.LdflFrame
import dev.ladoflow.display.protocol.VideoCodec
import dev.ladoflow.display.protocol.VideoFrameMetadata
import dev.ladoflow.display.protocol.VideoFramePayload
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
class AndroidMediaCodecVideoDecoderTest {
    @Test
    fun realMediaCodecRecoversAcrossSurfaceReplacementOnFreshKeyframe() = runBlocking {
        val decoder = AndroidMediaCodecVideoDecoder()
        val firstTexture = SurfaceTexture(false).apply { setDefaultBufferSize(64, 64) }
        val firstSurface = Surface(firstTexture)
        var secondTexture: SurfaceTexture? = null
        var secondSurface: Surface? = null
        val configuration = DisplayConfigPayload(
            width = 64,
            height = 64,
            refreshMillihz = 30_000u,
            bitrateKbps = 100u,
            codec = VideoCodec.H264,
            profile = CodecProfile.H264Main,
        )

        try {
            assertNull(decoder.configurationRejectionReason(configuration))

            decoder.setOutputSurface(firstSurface)
            decoder.applyConfiguration(configuration)
            submitKeyframeBurst(decoder, firstFrameId = 101u, firstTimestampMicros = 33_333u)
            withTimeout(10_000) {
                decoder.metrics.first { it.outputsReleasedToSurface >= 1L }
            }

            decoder.setOutputSurface(null)
            withTimeout(5_000) {
                decoder.state.first { it is VideoDecoderState.AwaitingSurface }
            }
            val firstSurfaceMetrics = decoder.metrics.value
            assertTrue(firstSurfaceMetrics.lastReleasedFrameId in 101uL..104uL)

            secondTexture = SurfaceTexture(false).apply { setDefaultBufferSize(64, 64) }
            secondSurface = Surface(secondTexture)
            decoder.setOutputSurface(secondSurface)
            withTimeout(5_000) {
                decoder.state.first { it is VideoDecoderState.AwaitingKeyframe }
            }

            assertTrue(decoder.submit(videoFrame(201u, 166_665u, FrameFlags.None)))
            withTimeout(5_000) {
                decoder.metrics.first {
                    it.droppedFrames == firstSurfaceMetrics.droppedFrames + 1L
                }
            }
            submitKeyframeBurst(decoder, firstFrameId = 202u, firstTimestampMicros = 199_998u)
            val finalMetrics = withTimeout(10_000) {
                decoder.metrics.first {
                    it.outputsReleasedToSurface > firstSurfaceMetrics.outputsReleasedToSurface
                }
            }

            assertTrue(finalMetrics.lastReleasedFrameId in 202uL..205uL)
            assertEquals(firstSurfaceMetrics.droppedFrames + 1L, finalMetrics.droppedFrames)
            assertTrue(finalMetrics.queueDepth in 0..8)
            assertTrue(finalMetrics.lastDecodeDurationMicros != null)
            assertTrue(finalMetrics.lastDecodeDurationMicros!! <= 60_000_000u)
            assertTrue(decoder.state.value is VideoDecoderState.Running)
        } finally {
            decoder.close()
            withTimeout(5_000) { decoder.state.first { it == VideoDecoderState.Closed } }
            secondSurface?.release()
            secondTexture?.release()
            firstSurface.release()
            firstTexture.release()
        }
    }

    private fun submitKeyframeBurst(
        decoder: AndroidMediaCodecVideoDecoder,
        firstFrameId: ULong,
        firstTimestampMicros: ULong,
    ) {
        repeat(4) { offset ->
            assertTrue(
                decoder.submit(
                    videoFrame(
                        frameId = firstFrameId + offset.toUInt(),
                        presentationTimestampMicros = firstTimestampMicros +
                            (offset.toUInt() * 33_333u),
                        flags = FrameFlags.Keyframe,
                    ),
                ),
            )
        }
    }

    private fun videoFrame(
        frameId: ULong,
        presentationTimestampMicros: ULong,
        flags: FrameFlags,
    ): LdflFrame = LdflFrame.fromPayload(
        flags = flags,
        sequence = frameId,
        payload = VideoFramePayload(
            metadata = VideoFrameMetadata(
                frameId = frameId,
                captureTimestampMicros = presentationTimestampMicros - 1_000u,
                presentationTimestampMicros = presentationTimestampMicros,
                durationMicros = 33_333u,
            ),
            accessUnit = Base64.decode(H264_MAIN_64X64_IDR_BASE64, Base64.DEFAULT),
        ),
    )

    companion object {
        /** Synthetic 64x64 red IDR, H.264 Main 3.1, Annex-B, B=0, ref=1, zero latency. */
        private const val H264_MAIN_64X64_IDR_BASE64 =
            "AAAAAQkQAAAAAWdNQB/cQmwEQAAAAwBAAAAPI8YM4AAAAAFo7g8sgAAAAQYF//9Z3EXpvebZSLeWLNgg2SPu73gyNjQgLSBjb3Jl" +
            "IDE2NCByMzE5MiBjMjRlMDZjIC0gSC4yNjQvTVBFRy00IEFWQyBjb2RlYyAtIENvcHlsZWZ0IDIwMDMtMjAyNCAtIGh0dHA6Ly93" +
            "d3cudmlkZW9sYW4ub3JnL3gyNjQuaHRtbCAtIG9wdGlvbnM6IGNhYmFjPTEgcmVmPTEgZGVibG9jaz0xOjA6MCBhbmFseXNlPTB4" +
            "MToweDExMSBtZT1oZXggc3VibWU9NyBwc3k9MSBwc3lfcmQ9MS4wMDowLjAwIG1peGVkX3JlZj0wIG1lX3JhbmdlPTE2IGNocm9t" +
            "YV9tZT0xIHRyZWxsaXM9MSA4eDhkY3Q9MCBjcW09MCBkZWFkem9uZT0yMSwxMSBmYXN0X3Bza2lwPTEgY2hyb21hX3FwX29mZnNl" +
            "dD0tMiB0aHJlYWRzPTEgbG9va2FoZWFkX3RocmVhZHM9MSBzbGljZWRfdGhyZWFkcz0wIG5yPTAgZGVjaW1hdGU9MSBpbnRlcmxh" +
            "Y2VkPTAgYmx1cmF5X2NvbXBhdD0wIGNvbnN0cmFpbmVkX2ludHJhPTAgYmZyYW1lcz0wIHdlaWdodHA9MCBrZXlpbnQ9MSBrZXlp" +
            "bnRfbWluPTEgc2NlbmVjdXQ9MCBpbnRyYV9yZWZyZXNoPTAgcmM9Y3JmIG1idHJlZT0wIGNyZj0yMy4wIHFjb21wPTAuNjAgcXBt" +
            "aW49MCBxcG1heD02OSBxcHN0ZXA9NCBpcF9yYXRpbz0xLjQwIGFxPTE6MS4wMACAAAABZYiEBL/+6Mn8yysv49Q3s0Yps0mnHM" +
            "I9HSP+uQdmS8ghwVkHi4nB"
    }
}
