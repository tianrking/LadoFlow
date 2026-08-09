package dev.ladoflow.display.transport.usb

import dev.ladoflow.display.protocol.IncrementalFrameDecoder
import dev.ladoflow.display.protocol.LdflFrame
import dev.ladoflow.display.protocol.LdflProtocolException
import dev.ladoflow.display.protocol.MessageType
import dev.ladoflow.display.protocol.MonotonicSequenceValidator
import java.io.Closeable
import java.io.InputStream
import java.io.OutputStream
import java.util.concurrent.atomic.AtomicBoolean
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineDispatcher
import kotlinx.coroutines.CoroutineName
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.receiveAsFlow
import kotlinx.coroutines.launch

interface UsbDuplexConnection : Closeable {
    val input: InputStream
    val output: OutputStream
}

enum class UsbIoFailureKind {
    EndOfStream,
    Io,
    Protocol,
}

sealed interface UsbIoSessionState {
    data object Idle : UsbIoSessionState

    data object Running : UsbIoSessionState

    data object Closed : UsbIoSessionState

    data class Failed(
        val kind: UsbIoFailureKind,
        val message: String,
    ) : UsbIoSessionState
}

class UsbIoSession(
    private val connection: UsbDuplexConnection,
    parentScope: CoroutineScope,
    ioDispatcher: CoroutineDispatcher = Dispatchers.IO,
    readBufferBytes: Int = USB_READ_BUFFER_BYTES,
    inboundQueueCapacity: Int = 8,
    outboundQueueCapacity: Int = 64,
) : Closeable {
    private val sessionJob = SupervisorJob(parentScope.coroutineContext[Job])
    private val scope = CoroutineScope(
        parentScope.coroutineContext + sessionJob + ioDispatcher + CoroutineName("LadoFlow USB I/O"),
    )
    private val decoder = IncrementalFrameDecoder()
    private val inboundSequences = MonotonicSequenceValidator()
    private val inbound = Channel<LdflFrame>(inboundQueueCapacity)
    private val controlOutbound = Channel<LdflFrame>(outboundQueueCapacity)
    private val started = AtomicBoolean(false)
    private val terminated = AtomicBoolean(false)
    private val mutableState = MutableStateFlow<UsbIoSessionState>(UsbIoSessionState.Idle)
    private val readBufferBytes = readBufferBytes

    val state: StateFlow<UsbIoSessionState> = mutableState.asStateFlow()
    val frames: Flow<LdflFrame> = inbound.receiveAsFlow()

    init {
        require(readBufferBytes > 0)
        require(inboundQueueCapacity > 0)
        require(outboundQueueCapacity > 0)
    }

    fun start() {
        check(started.compareAndSet(false, true)) { "USB I/O session was already started" }
        mutableState.value = UsbIoSessionState.Running
        scope.launch { readLoop() }
        scope.launch { writeLoop() }
    }

    suspend fun sendControl(frame: LdflFrame) {
        require(frame.messageType != MessageType.VideoFrame) {
            "Android display transport does not send video frames"
        }
        controlOutbound.send(frame)
    }

    fun trySendControl(frame: LdflFrame): Boolean {
        require(frame.messageType != MessageType.VideoFrame) {
            "Android display transport does not send video frames"
        }
        return controlOutbound.trySend(frame).isSuccess
    }

    private suspend fun readLoop() {
        val readBuffer = ByteArray(readBufferBytes)
        try {
            while (true) {
                val count = connection.input.read(readBuffer)
                if (count < 0) {
                    if (decoder.bufferedBytes > 0) {
                        terminate(
                            UsbIoFailureKind.Protocol,
                            "USB accessory ended with ${decoder.bufferedBytes} trailing LDFL bytes",
                        )
                    } else {
                        terminate(
                            UsbIoFailureKind.EndOfStream,
                            "USB accessory reached end of stream",
                        )
                    }
                    return
                }
                if (count == 0) continue
                val frames = decoder.push(readBuffer.copyOf(count))
                frames.forEach { frame ->
                    inboundSequences.observe(frame.sequence)
                    inbound.send(frame)
                }
            }
        } catch (cancelled: CancellationException) {
            throw cancelled
        } catch (protocol: LdflProtocolException) {
            terminate(UsbIoFailureKind.Protocol, protocol.message ?: "Invalid LDFL stream")
        } catch (exception: Exception) {
            terminate(UsbIoFailureKind.Io, exception.message ?: "USB read failed")
        }
    }

    private suspend fun writeLoop() {
        try {
            for (frame in controlOutbound) {
                connection.output.write(frame.encode())
                connection.output.flush()
            }
        } catch (cancelled: CancellationException) {
            throw cancelled
        } catch (exception: Exception) {
            terminate(UsbIoFailureKind.Io, exception.message ?: "USB write failed")
        }
    }

    private fun terminate(kind: UsbIoFailureKind, message: String) {
        if (!terminated.compareAndSet(false, true)) return
        mutableState.value = UsbIoSessionState.Failed(kind, message)
        closeResources()
    }

    override fun close() {
        if (!terminated.compareAndSet(false, true)) return
        mutableState.value = UsbIoSessionState.Closed
        closeResources()
    }

    private fun closeResources() {
        inbound.close()
        controlOutbound.close()
        runCatching { connection.close() }
        sessionJob.cancel()
    }
}
