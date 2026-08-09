package dev.ladoflow.display.session

import dev.ladoflow.display.protocol.ButtonState
import dev.ladoflow.display.protocol.InputPayload
import dev.ladoflow.display.protocol.KeyInput
import dev.ladoflow.display.protocol.KeyModifiers
import dev.ladoflow.display.protocol.LdflFrame
import dev.ladoflow.display.protocol.MessageType
import dev.ladoflow.display.protocol.PingPayload
import dev.ladoflow.display.protocol.PointerMoveInput
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.async
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

@OptIn(ExperimentalCoroutinesApi::class)
class SessionOutboundMuxTest {
    @Test
    fun prioritySelectionHappensBeforeMonotonicSequenceAssignment() = runTest {
        val firstSendEntered = CompletableDeferred<Unit>()
        val releaseFirstSend = CompletableDeferred<Unit>()
        val fourFramesSent = CompletableDeferred<Unit>()
        val sent = mutableListOf<LdflFrame>()
        val mux = SessionOutboundMux(
            parentScope = backgroundScope,
            sendFrame = { frame ->
                if (sent.isEmpty()) {
                    firstSendEntered.complete(Unit)
                    releaseFirstSend.await()
                }
                sent += frame
                if (sent.size == 4) fourFramesSent.complete(Unit)
                true
            },
            onSendFailure = { error("unexpected failure: $it") },
        )
        val firstMotion = InputPayload(1u, PointerMoveInput(10, 20))
        val secondMotion = InputPayload(2u, PointerMoveInput(30, 40))
        val criticalKey = InputPayload(
            3u,
            KeyInput(usage = 0x04, state = ButtonState.Pressed, modifiers = KeyModifiers.None),
        )

        assertTrue(mux.trySendCoalescibleInput(firstMotion))
        firstSendEntered.await()
        assertTrue(mux.trySendCoalescibleInput(secondMotion))
        val criticalSend = async { mux.sendCriticalInput(criticalKey) }
        val controlSend = async { mux.sendControl(PingPayload(4u, 5u)) }
        criticalSend.await()
        controlSend.await()
        releaseFirstSend.complete(Unit)
        fourFramesSent.await()

        assertEquals(
            listOf(MessageType.Input, MessageType.Ping, MessageType.Input, MessageType.Input),
            sent.map { it.messageType },
        )
        assertEquals(listOf(0uL, 1uL, 2uL, 3uL), sent.map { it.sequence })
        assertEquals(firstMotion, sent[0].decodePayload())
        assertEquals(criticalKey, sent[2].decodePayload())
        assertEquals(secondMotion, sent[3].decodePayload())
        mux.close()
    }
}
