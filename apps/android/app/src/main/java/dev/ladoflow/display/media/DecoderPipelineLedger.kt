package dev.ladoflow.display.media

import dev.ladoflow.display.protocol.MAX_STAGE_DURATION_MICROS
import java.util.ArrayDeque
import java.util.LinkedHashMap
import java.util.concurrent.atomic.AtomicInteger

/** Thread-safe capacity reservation spanning caller, Handler, and MediaCodec stages. */
internal class DecoderInFlightWindow(
    private val capacity: Int,
) {
    private val occupied = AtomicInteger(0)

    init {
        require(capacity > 0) { "Decoder in-flight capacity must be positive" }
    }

    val depth: Int
        get() = occupied.get()

    fun tryAcquire(): Boolean {
        while (true) {
            val current = occupied.get()
            if (current >= capacity) return false
            if (occupied.compareAndSet(current, current + 1)) return true
        }
    }

    fun release(count: Int = 1): Int {
        require(count >= 0) { "Released access-unit count must not be negative" }
        if (count == 0) return depth
        val remaining = occupied.addAndGet(-count)
        check(remaining >= 0) { "Released more decoder access units than were reserved" }
        return remaining
    }
}

internal data class CorrelatedDecoderOutput(
    val frameId: ULong,
    val decodeDurationMicros: UInt,
)

/**
 * Tracks access units after they reach the decoder Handler until MediaCodec
 * releases or discards them. [DecoderInFlightWindow] applies the outer bound
 * before Handler submission.
 *
 * The bound covers pending input and inputs already queued into MediaCodec, so
 * a fast producer cannot hide an unbounded backlog behind codec callbacks.
 */
internal class DecoderPipelineLedger(
    private val capacity: Int,
) {
    private val pending = ArrayDeque<H264DecoderInput>(capacity)
    private val queuedByTimestamp = LinkedHashMap<Long, ArrayDeque<QueuedFrame>>()

    init {
        require(capacity > 0) { "Decoder pipeline capacity must be positive" }
    }

    val depth: Int
        get() = pending.size + queuedByTimestamp.values.sumOf { it.size }

    fun enqueue(input: H264DecoderInput): Boolean {
        if (depth >= capacity) return false
        pending.addLast(input)
        return true
    }

    fun takePending(): H264DecoderInput? = pending.pollFirst()

    fun markQueued(
        input: H264DecoderInput,
        queuedAtNanos: Long,
    ) {
        queuedByTimestamp
            .getOrPut(input.presentationTimestampMicros) { ArrayDeque() }
            .addLast(QueuedFrame(input.frameId, queuedAtNanos))
    }

    fun takeOutput(
        presentationTimestampMicros: Long,
        outputAvailableAtNanos: Long,
    ): CorrelatedDecoderOutput? {
        val frames = queuedByTimestamp[presentationTimestampMicros] ?: return null
        val frame = frames.pollFirst() ?: return null
        if (frames.isEmpty()) queuedByTimestamp.remove(presentationTimestampMicros)
        val elapsedMicros = ((outputAvailableAtNanos - frame.queuedAtNanos).coerceAtLeast(0L) / 1_000L)
            .coerceAtMost(MAX_STAGE_DURATION_MICROS.toLong())
            .toUInt()
        return CorrelatedDecoderOutput(frame.frameId, elapsedMicros)
    }

    fun discardAll(): Int {
        val count = depth
        pending.clear()
        queuedByTimestamp.clear()
        return count
    }

    private data class QueuedFrame(
        val frameId: ULong,
        val queuedAtNanos: Long,
    )
}
