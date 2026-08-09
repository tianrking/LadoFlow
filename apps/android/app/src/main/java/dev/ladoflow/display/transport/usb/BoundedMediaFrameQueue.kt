package dev.ladoflow.display.transport.usb

import dev.ladoflow.display.protocol.FrameFlags
import dev.ladoflow.display.protocol.LdflFrame
import dev.ladoflow.display.protocol.MessageType
import java.io.Closeable
import java.util.ArrayDeque
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.flow

internal enum class MediaOfferResult {
    Enqueued,
    ReplacedWithKeyframe,
    DroppedAwaitingKeyframe,
    OverflowedAwaitingKeyframe,
}

internal class BoundedMediaFrameQueue(
    private val capacity: Int,
) : Closeable {
    private val frames = ArrayDeque<LdflFrame>(capacity)
    private val available = Channel<Unit>(Channel.CONFLATED)
    private val lock = Any()
    private var closed = false
    private var awaitingKeyframe = false
    private var mutableDroppedFrames = 0L

    init {
        require(capacity > 0)
    }

    val droppedFrames: Long
        get() = synchronized(lock) { mutableDroppedFrames }

    fun offer(frame: LdflFrame): MediaOfferResult {
        require(frame.messageType == MessageType.VideoFrame)
        val keyframe = frame.flags.contains(FrameFlags.Keyframe)
        val result = synchronized(lock) {
            check(!closed) { "Media queue is closed" }
            when {
                keyframe -> {
                    val replaced = frames.size
                    frames.clear()
                    frames.addLast(frame)
                    mutableDroppedFrames += replaced.toLong()
                    awaitingKeyframe = false
                    if (replaced > 0) {
                        MediaOfferResult.ReplacedWithKeyframe
                    } else {
                        MediaOfferResult.Enqueued
                    }
                }

                awaitingKeyframe -> {
                    mutableDroppedFrames += 1
                    MediaOfferResult.DroppedAwaitingKeyframe
                }

                frames.size >= capacity -> {
                    mutableDroppedFrames += frames.size.toLong() + 1
                    frames.clear()
                    awaitingKeyframe = true
                    MediaOfferResult.OverflowedAwaitingKeyframe
                }

                else -> {
                    frames.addLast(frame)
                    MediaOfferResult.Enqueued
                }
            }
        }
        if (result == MediaOfferResult.Enqueued || result == MediaOfferResult.ReplacedWithKeyframe) {
            available.trySend(Unit)
        }
        return result
    }

    fun asFlow(): Flow<LdflFrame> = flow {
        while (true) {
            val next = synchronized(lock) { frames.pollFirst() }
            if (next != null) {
                emit(next)
                continue
            }
            val signal = available.receiveCatching()
            if (signal.isClosed) return@flow
        }
    }

    override fun close() {
        synchronized(lock) {
            if (closed) return
            closed = true
        }
        available.close()
    }
}
