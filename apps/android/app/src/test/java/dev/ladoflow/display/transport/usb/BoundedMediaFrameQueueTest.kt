package dev.ladoflow.display.transport.usb

import dev.ladoflow.display.protocol.FrameFlags
import dev.ladoflow.display.protocol.LdflFrame
import dev.ladoflow.display.protocol.MessageType
import kotlinx.coroutines.async
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Test

class BoundedMediaFrameQueueTest {
    @Test
    fun `overflow drops the broken delta chain until a keyframe arrives`() = runTest {
        val queue = BoundedMediaFrameQueue(capacity = 2)

        assertEquals(MediaOfferResult.Enqueued, queue.offer(videoFrame(1, keyframe = false)))
        assertEquals(MediaOfferResult.Enqueued, queue.offer(videoFrame(2, keyframe = false)))
        assertEquals(
            MediaOfferResult.OverflowedAwaitingKeyframe,
            queue.offer(videoFrame(3, keyframe = false)),
        )
        assertEquals(
            MediaOfferResult.DroppedAwaitingKeyframe,
            queue.offer(videoFrame(4, keyframe = false)),
        )

        val expected = videoFrame(5, keyframe = true)
        assertEquals(MediaOfferResult.Enqueued, queue.offer(expected))
        val received = async { queue.asFlow().first() }

        assertEquals(expected, received.await())
        assertEquals(4, queue.droppedFrames)
        queue.close()
    }

    @Test
    fun `new keyframe supersedes queued stale media`() = runTest {
        val queue = BoundedMediaFrameQueue(capacity = 3)
        queue.offer(videoFrame(1, keyframe = true))
        queue.offer(videoFrame(2, keyframe = false))
        val replacement = videoFrame(3, keyframe = true)

        assertEquals(MediaOfferResult.ReplacedWithKeyframe, queue.offer(replacement))
        assertEquals(replacement, queue.asFlow().first())
        assertEquals(2, queue.droppedFrames)
        queue.close()
    }

    private fun videoFrame(sequence: Long, keyframe: Boolean): LdflFrame = LdflFrame(
        messageType = MessageType.VideoFrame,
        flags = if (keyframe) FrameFlags.Keyframe else FrameFlags.None,
        sequence = sequence.toULong(),
        payload = byteArrayOf(sequence.toByte()),
    )
}
