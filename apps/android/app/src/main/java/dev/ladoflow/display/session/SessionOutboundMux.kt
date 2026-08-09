package dev.ladoflow.display.session

import dev.ladoflow.display.protocol.FrameFlags
import dev.ladoflow.display.protocol.InputPayload
import dev.ladoflow.display.protocol.LdflFrame
import dev.ladoflow.display.protocol.LdflPayload
import java.io.Closeable
import java.util.concurrent.atomic.AtomicBoolean
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineName
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.launch
import kotlinx.coroutines.selects.select

/**
 * Selects payload priority before assigning the global sender sequence.
 *
 * The downstream USB writer remains one FIFO, so wire-order sequence numbers
 * are strictly increasing even when control overtakes queued motion input.
 */
internal class SessionOutboundMux(
    parentScope: CoroutineScope,
    private val sendFrame: suspend (LdflFrame) -> Boolean,
    private val onSendFailure: (String) -> Unit,
    controlCapacity: Int = 32,
    criticalInputCapacity: Int = 64,
    coalescibleInputCapacity: Int = 32,
) : Closeable {
    private val job = SupervisorJob(parentScope.coroutineContext[Job])
    private val scope = CoroutineScope(
        parentScope.coroutineContext + job + CoroutineName("LadoFlow outbound mux"),
    )
    private val control = Channel<OutboundPayload>(controlCapacity)
    private val criticalInput = Channel<OutboundPayload>(criticalInputCapacity)
    private val coalescibleInput = Channel<OutboundPayload>(coalescibleInputCapacity)
    private val closed = AtomicBoolean(false)
    private var nextSequence = 0uL
    private var sequenceExhausted = false

    init {
        require(controlCapacity > 0)
        require(criticalInputCapacity > 0)
        require(coalescibleInputCapacity > 0)
        scope.launch { writeLoop() }
    }

    suspend fun sendControl(
        payload: LdflPayload,
        flags: FrameFlags = FrameFlags.None,
    ) {
        control.send(OutboundPayload(payload, flags))
    }

    suspend fun sendCriticalInput(payload: InputPayload) {
        criticalInput.send(OutboundPayload(payload, FrameFlags.None))
    }

    fun trySendCoalescibleInput(payload: InputPayload): Boolean =
        coalescibleInput.trySend(OutboundPayload(payload, FrameFlags.None)).isSuccess

    private suspend fun writeLoop() {
        try {
            while (true) {
                val outbound = receiveNext() ?: return
                if (sequenceExhausted) {
                    onSendFailure("LDFL sender sequence exhausted")
                    return
                }
                val sequence = nextSequence
                if (nextSequence == ULong.MAX_VALUE) {
                    sequenceExhausted = true
                } else {
                    nextSequence += 1uL
                }
                val frame = LdflFrame.fromPayload(
                    flags = outbound.flags,
                    sequence = sequence,
                    payload = outbound.payload,
                )
                if (!sendFrame(frame)) {
                    onSendFailure("USB session closed before an outbound LDFL frame was written")
                    return
                }
            }
        } catch (cancelled: CancellationException) {
            throw cancelled
        } catch (exception: Exception) {
            onSendFailure(exception.message ?: "Outbound LDFL writer failed")
        }
    }

    private suspend fun receiveNext(): OutboundPayload? {
        val closedLanes = mutableSetOf<OutboundLane>()
        while (true) {
            receiveAvailable(control, OutboundLane.Control, closedLanes)?.let { return it }
            receiveAvailable(criticalInput, OutboundLane.CriticalInput, closedLanes)?.let { return it }
            receiveAvailable(coalescibleInput, OutboundLane.CoalescibleInput, closedLanes)
                ?.let { return it }
            if (closedLanes.size == OutboundLane.entries.size) return null

            val selection = select<OutboundSelection> {
                if (OutboundLane.Control !in closedLanes) {
                    control.onReceiveCatching { result ->
                        OutboundSelection(OutboundLane.Control, result.getOrNull())
                    }
                }
                if (OutboundLane.CriticalInput !in closedLanes) {
                    criticalInput.onReceiveCatching { result ->
                        OutboundSelection(OutboundLane.CriticalInput, result.getOrNull())
                    }
                }
                if (OutboundLane.CoalescibleInput !in closedLanes) {
                    coalescibleInput.onReceiveCatching { result ->
                        OutboundSelection(OutboundLane.CoalescibleInput, result.getOrNull())
                    }
                }
            }
            selection.payload?.let { return it }
            closedLanes += selection.lane
        }
    }

    private fun receiveAvailable(
        channel: Channel<OutboundPayload>,
        lane: OutboundLane,
        closedLanes: MutableSet<OutboundLane>,
    ): OutboundPayload? {
        if (lane in closedLanes) return null
        val result = channel.tryReceive()
        if (result.isClosed) closedLanes += lane
        return result.getOrNull()
    }

    override fun close() {
        if (!closed.compareAndSet(false, true)) return
        control.close()
        criticalInput.close()
        coalescibleInput.close()
        job.cancel()
    }

    private data class OutboundPayload(
        val payload: LdflPayload,
        val flags: FrameFlags,
    )

    private enum class OutboundLane {
        Control,
        CriticalInput,
        CoalescibleInput,
    }

    private data class OutboundSelection(
        val lane: OutboundLane,
        val payload: OutboundPayload?,
    )
}
