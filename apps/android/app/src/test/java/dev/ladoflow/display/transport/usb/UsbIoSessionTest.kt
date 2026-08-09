package dev.ladoflow.display.transport.usb

import dev.ladoflow.display.protocol.FrameFlags
import dev.ladoflow.display.protocol.IncrementalFrameDecoder
import dev.ladoflow.display.protocol.InputPayload
import dev.ladoflow.display.protocol.LdflFrame
import dev.ladoflow.display.protocol.MessageType
import dev.ladoflow.display.protocol.PingPayload
import dev.ladoflow.display.protocol.PointerMoveInput
import java.io.ByteArrayOutputStream
import java.io.InputStream
import java.io.OutputStream
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.async
import kotlinx.coroutines.flow.toList
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.test.UnconfinedTestDispatcher
import kotlinx.coroutines.test.advanceUntilIdle
import kotlinx.coroutines.test.runTest
import kotlinx.coroutines.withTimeout
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

@OptIn(ExperimentalCoroutinesApi::class)
class UsbIoSessionTest {
    @Test
    fun `reader handles split and coalesced LDFL frames`() = runTest {
        val control = LdflFrame.fromPayload(FrameFlags.None, 1uL, PingPayload(7uL, 8uL))
        val media = LdflFrame(
            MessageType.VideoFrame,
            FrameFlags.Keyframe,
            2uL,
            byteArrayOf(1, 2, 3),
        )
        val bytes = control.encode() + media.encode()
        val input = ChunkedInputStream(
            listOf(
                bytes.copyOfRange(0, 3),
                bytes.copyOfRange(3, control.encode().size + 7),
                bytes.copyOfRange(control.encode().size + 7, bytes.size),
            ),
        )
        val connection = TestConnection(input, ByteArrayOutputStream())
        val dispatcher = UnconfinedTestDispatcher(testScheduler)
        val session = UsbIoSession(connection, backgroundScope, dispatcher, readBufferBytes = 8)
        val controls = async(dispatcher) { session.controlFrames.toList() }
        val mediaFrames = async(dispatcher) { session.mediaFrames.toList() }

        session.start()
        advanceUntilIdle()

        assertEquals(listOf(control), controls.await())
        assertEquals(listOf(media), mediaFrames.await())
        assertTrue(session.state.value is UsbIoSessionState.Failed)
        assertEquals(
            UsbIoFailureKind.EndOfStream,
            (session.state.value as UsbIoSessionState.Failed).kind,
        )
    }

    @Test
    fun `writer emits one complete encoded control frame`() = runBlocking {
        val input = BlockingInputStream()
        val output = SignallingOutputStream()
        val connection = TestConnection(input, output)
        val session = UsbIoSession(connection, this)
        val frame = LdflFrame.fromPayload(FrameFlags.AckRequired, 9uL, PingPayload(10uL, 11uL))

        session.start()
        session.sendControl(frame)
        withTimeout(5_000) { output.firstWrite.await() }

        assertArrayEquals(frame.encode(), output.bytes())
        session.close()
    }

    @Test
    fun `writer prioritizes queued control ahead of queued input`() = runBlocking {
        val input = BlockingInputStream()
        val output = FirstWriteGatedOutputStream()
        val session = UsbIoSession(TestConnection(input, output), this)
        val firstInput = inputFrame(sequence = 1u, x = 10, y = 20)
        val secondInput = inputFrame(sequence = 2u, x = 30, y = 40)
        val control = LdflFrame.fromPayload(FrameFlags.None, 3u, PingPayload(50u, 60u))

        session.start()
        session.sendControl(firstInput)
        assertTrue(output.firstWriteEntered.await(5, TimeUnit.SECONDS))
        session.sendControl(secondInput)
        session.sendControl(control)
        output.releaseFirstWrite.countDown()
        assertTrue(output.threeWrites.await(5, TimeUnit.SECONDS))

        val decoded = IncrementalFrameDecoder().push(output.bytes())
        assertEquals(listOf(firstInput, control, secondInput), decoded)
        session.close()
    }

    @Test
    fun `close closes connection to release a blocking accessory read`() = runBlocking {
        val input = CloseObservedInputStream()
        val session = UsbIoSession(
            TestConnection(input, ByteArrayOutputStream()),
            this,
        )

        session.start()
        assertTrue(input.readEntered.await(5, TimeUnit.SECONDS))
        session.close()

        assertTrue(input.closed.await(5, TimeUnit.SECONDS))
        assertEquals(UsbIoSessionState.Closed, session.state.value)
    }

    @Test(expected = IllegalArgumentException::class)
    fun `display endpoint refuses outbound video frames`() = runBlocking {
        val session = UsbIoSession(
            TestConnection(BlockingInputStream(), ByteArrayOutputStream()),
            this,
        )
        session.sendControl(
            LdflFrame(MessageType.VideoFrame, FrameFlags.None, 1uL, byteArrayOf(1)),
        )
    }

    private fun inputFrame(
        sequence: ULong,
        x: Int,
        y: Int,
    ): LdflFrame = LdflFrame.fromPayload(
        flags = FrameFlags.None,
        sequence = sequence,
        payload = InputPayload(sequence * 1_000u, PointerMoveInput(x, y)),
    )
}

private class TestConnection(
    override val input: InputStream,
    override val output: OutputStream,
) : UsbDuplexConnection {
    override fun close() {
        input.close()
        output.close()
    }
}

private class ChunkedInputStream(
    private val chunks: List<ByteArray>,
) : InputStream() {
    private var chunkIndex = 0
    private var offset = 0

    override fun read(): Int {
        val one = ByteArray(1)
        return if (read(one, 0, 1) < 0) -1 else one[0].toInt() and 0xff
    }

    override fun read(target: ByteArray, targetOffset: Int, length: Int): Int {
        if (chunkIndex >= chunks.size) return -1
        val chunk = chunks[chunkIndex]
        val count = minOf(length, chunk.size - offset)
        chunk.copyInto(target, targetOffset, offset, offset + count)
        offset += count
        if (offset == chunk.size) {
            chunkIndex += 1
            offset = 0
        }
        return count
    }
}

private class BlockingInputStream : InputStream() {
    private val closed = CountDownLatch(1)

    override fun read(): Int {
        closed.await(30, TimeUnit.SECONDS)
        return -1
    }

    override fun read(target: ByteArray, offset: Int, length: Int): Int = read()

    override fun close() {
        closed.countDown()
    }
}

private class CloseObservedInputStream : InputStream() {
    val readEntered = CountDownLatch(1)
    val closed = CountDownLatch(1)

    override fun read(): Int {
        readEntered.countDown()
        closed.await(30, TimeUnit.SECONDS)
        return -1
    }

    override fun read(target: ByteArray, offset: Int, length: Int): Int = read()

    override fun close() {
        closed.countDown()
    }
}

private class SignallingOutputStream : OutputStream() {
    private val delegate = ByteArrayOutputStream()
    val firstWrite = CompletableDeferred<Unit>()

    @Synchronized
    override fun write(value: Int) {
        delegate.write(value)
        firstWrite.complete(Unit)
    }

    @Synchronized
    override fun write(bytes: ByteArray, offset: Int, length: Int) {
        delegate.write(bytes, offset, length)
        firstWrite.complete(Unit)
    }

    @Synchronized
    fun bytes(): ByteArray = delegate.toByteArray()
}

private class FirstWriteGatedOutputStream : OutputStream() {
    private val delegate = ByteArrayOutputStream()
    private var writes = 0
    val firstWriteEntered = CountDownLatch(1)
    val releaseFirstWrite = CountDownLatch(1)
    val threeWrites = CountDownLatch(1)

    override fun write(value: Int) {
        write(byteArrayOf(value.toByte()), 0, 1)
    }

    @Synchronized
    override fun write(bytes: ByteArray, offset: Int, length: Int) {
        if (writes == 0) {
            firstWriteEntered.countDown()
            releaseFirstWrite.await(5, TimeUnit.SECONDS)
        }
        delegate.write(bytes, offset, length)
        writes += 1
        if (writes >= 3) threeWrites.countDown()
    }

    @Synchronized
    fun bytes(): ByteArray = delegate.toByteArray()

    override fun close() {
        releaseFirstWrite.countDown()
    }
}
