package dev.ladoflow.display.media

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class DecoderPipelineLedgerTest {
    @Test
    fun admissionWindowBoundsFramesBeforeHandlerAndReleasesBatches() {
        val window = DecoderInFlightWindow(capacity = 3)

        assertTrue(window.tryAcquire())
        assertTrue(window.tryAcquire())
        assertTrue(window.tryAcquire())
        assertFalse(window.tryAcquire())
        assertEquals(3, window.depth)

        assertEquals(1, window.release(2))
        assertTrue(window.tryAcquire())
        assertTrue(window.tryAcquire())
        assertFalse(window.tryAcquire())
        assertEquals(3, window.depth)
    }

    @Test
    fun boundIncludesInputsAlreadyQueuedIntoCodec() {
        val ledger = DecoderPipelineLedger(capacity = 3)
        val first = input(frameId = 1u, presentationTimestampMicros = 10_000)
        val second = input(frameId = 2u, presentationTimestampMicros = 20_000)
        val third = input(frameId = 3u, presentationTimestampMicros = 30_000)

        assertTrue(ledger.enqueue(first))
        ledger.markQueued(requireNotNull(ledger.takePending()), queuedAtNanos = 1_000_000)
        assertTrue(ledger.enqueue(second))
        ledger.markQueued(requireNotNull(ledger.takePending()), queuedAtNanos = 2_000_000)
        assertTrue(ledger.enqueue(third))

        assertEquals(3, ledger.depth)
        assertFalse(ledger.enqueue(input(frameId = 4u, presentationTimestampMicros = 40_000)))
        assertEquals(3, ledger.depth)
    }

    @Test
    fun duplicatePresentationTimestampsCorrelateInInputOrder() {
        val ledger = DecoderPipelineLedger(capacity = 3)
        listOf(10uL, 11uL).forEachIndexed { index, frameId ->
            assertTrue(ledger.enqueue(input(frameId, presentationTimestampMicros = 50_000)))
            ledger.markQueued(
                requireNotNull(ledger.takePending()),
                queuedAtNanos = 1_000_000L + index * 100_000L,
            )
        }

        val first = ledger.takeOutput(50_000, outputAvailableAtNanos = 2_500_000)
        val second = ledger.takeOutput(50_000, outputAvailableAtNanos = 3_500_000)

        assertEquals(10uL, first?.frameId)
        assertEquals(1_500u, first?.decodeDurationMicros)
        assertEquals(11uL, second?.frameId)
        assertEquals(2_400u, second?.decodeDurationMicros)
        assertEquals(0, ledger.depth)
        assertNull(ledger.takeOutput(50_000, outputAvailableAtNanos = 4_000_000))
    }

    @Test
    fun discardReportsEveryPendingAndCodecQueuedFrame() {
        val ledger = DecoderPipelineLedger(capacity = 3)
        assertTrue(ledger.enqueue(input(20u, 10_000)))
        ledger.markQueued(requireNotNull(ledger.takePending()), queuedAtNanos = 1_000)
        assertTrue(ledger.enqueue(input(21u, 20_000)))
        assertTrue(ledger.enqueue(input(22u, 30_000)))

        assertEquals(3, ledger.discardAll())
        assertEquals(0, ledger.depth)
    }

    private fun input(
        frameId: ULong,
        presentationTimestampMicros: Long,
    ) = H264DecoderInput(
        frameId = frameId,
        presentationTimestampMicros = presentationTimestampMicros,
        durationMicros = 16_667u,
        accessUnit = byteArrayOf(0, 0, 1, 0x65),
        isKeyframe = frameId == 1uL,
        endOfStream = false,
        containsIdr = frameId == 1uL,
    )
}
